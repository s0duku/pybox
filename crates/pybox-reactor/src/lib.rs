//! in-process python sandbox based on rustpython and WASM

mod exec;
mod ioctl;
mod mem;
mod protected;
mod sanitizer;

use libc::ssize_t;

use rustpython_vm::{Interpreter, PyObjectRef, pymodule};

use protected::ProtectedLocals;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ioctl::pybox_bytes;

struct PyboxState {
    pub locals: HashMap<String, (PyObjectRef, Rc<Interpreter>)>,
}

thread_local! {
    static PYBOX_STATE: RefCell<PyboxState> = RefCell::new(PyboxState{locals:HashMap::new()});
}

/// create a new default pybox interpreter
pub fn pybox_new_interpreter() -> Rc<Interpreter> {
    let builder = Interpreter::builder(Default::default())
        .add_frozen_modules(rustpython_pylib::FROZEN_STDLIB);
    let def = py_pybox::module_def(&builder.ctx);
    let interp = Rc::new(builder.add_native_module(def).build());

    // Register ProtectedDict type to Interpreter
    interp.enter(|vm| {
        use rustpython_vm::class::PyClassImpl;
        let protected_locals_type = ProtectedLocals::make_class(&vm.ctx);

        match (|| -> Result<(), String> {
            let _ = vm
                .builtins
                .set_attr("ProtectedLocals", protected_locals_type, vm)
                .map_err(|_| "Failed to register ProtectedLocals")?;

            let pybox_module = vm
                .import("pybox", 0)
                .map_err(|_| "Failed to import pybox module")?;

            let pybox_ioctl_host = pybox_module
                .get_attr("pybox_ioctl_host", vm)
                .map_err(|_| "Failed to import 'pybox_ioctl_host'")?;

            vm.builtins
                .set_attr("pybox_ioctl_host", pybox_ioctl_host, vm)
                .map_err(|_| "Failed to register 'pybox_ioctl_host'")?;

            let pybox_json_rpc = pybox_module
                .get_attr("pybox_json_rpc", vm)
                .map_err(|_| "Failed to import 'pybox_json_rpc'")?;

            vm.builtins
                .set_attr("pybox_json_rpc", pybox_json_rpc, vm)
                .map_err(|_| "Failed to register 'pybox_json_rpc'")?;

            // delete unsafe builtins
            sanitizer::builtins_sanitizer(vm)?;

            Ok(())
        })() {
            Ok(_) => (),
            Err(err) => {
                panic!("{}", &err);
            }
        }
    });

    interp
}

/// init one local execution enviroment in pybox
/// * `id` for
#[unsafe(no_mangle)]
pub extern "C" fn pybox_init_local(id: *const ioctl::pybox_bytes) -> ssize_t {
    PYBOX_STATE.with_borrow_mut(|pybox_state| {
        let Ok(id) = (unsafe { (*id).string() }) else {
            return -1;
        };

        // allocate a new interpreter for sys modules isolation
        let interpreter = pybox_new_interpreter();

        // create ProtectedLocals for vm
        let locals_obj = interpreter.enter(|vm| {
            // get the type
            let protected_locals_type = vm
                .builtins
                .get_attr("ProtectedLocals", vm)
                .expect("ProtectedLocals type not registered");

            // create instance
            protected_locals_type
                .call((), vm)
                .expect("Failed to create ProtectedLocals instance")
        });

        pybox_state
            .locals
            .insert(id.to_string(), (locals_obj, interpreter));

        0
    })
}

/// create a new local from existing local (shallow copy)
/// * `id` new local id
/// * `from_id` from local id
/// will not auto protect variables, caller make decision
#[unsafe(no_mangle)]
pub extern "C" fn pybox_init_local_from(
    id: *const ioctl::pybox_bytes,
    from_id: *const ioctl::pybox_bytes,
) -> ssize_t {
    PYBOX_STATE.with_borrow_mut(|pybox_state| {
        let Ok((id, from_id)) = (|| -> Result<_, ()> {
            unsafe {
                let id = (*id).string()?;
                let from_id = (*from_id).string()?;
                Ok((id, from_id))
            }
        })() else {
            return -1;
        };

        // exsist?
        if let Some(_) = pybox_state.locals.get(id) {
            return -1;
        }

        // from_id not exsist?
        let Some((from_local, _)) = pybox_state.locals.get(from_id) else {
            return -1;
        };

        // new interpreter
        let new_interpreter = pybox_new_interpreter();

        // copy the from_local dict to create new local
        let new_locals_obj = new_interpreter.enter(|vm| -> Result<PyObjectRef, ()> {
            // convert from_local to ProtectedLocals
            let from_protected = from_local.downcast_ref::<ProtectedLocals>().ok_or(())?;

            // create a new ProtectedLocals instance
            let protected_locals_type = vm
                .builtins
                .get_attr("ProtectedLocals", vm)
                .map_err(|_| ())?;

            let new_locals = protected_locals_type.call((), vm).map_err(|_| ())?;

            let new_protected = new_locals.downcast_ref::<ProtectedLocals>().ok_or(())?;

            // copy dict content
            let from_dict = from_protected.dict();
            for (key, value) in from_dict.into_iter() {
                new_protected
                    .dict()
                    .set_item(&*key, value, vm)
                    .map_err(|_| ())?;
            }

            // let caller do it
            // for protected_key in from_protected.get_protected_keys() {
            //     new_protected.protect(&protected_key);
            // }

            Ok(new_locals)
        });

        let Ok(new_locals_obj) = new_locals_obj else {
            return -1;
        };

        pybox_state
            .locals
            .insert(id.to_string(), (new_locals_obj, new_interpreter));

        0
    })
}

/// delete a local enviroment
/// * `id` local enviroment id
#[unsafe(no_mangle)]
pub extern "C" fn pybox_del_local(id: *const pybox_bytes) -> ssize_t {
    PYBOX_STATE.with_borrow_mut(|pybox_state| {
        let Ok(id) = (unsafe { (*id).string() }) else {
            return -1;
        };

        // no id?
        if !pybox_state.locals.contains_key(id) {
            return -1;
        }

        // deleted
        pybox_state.locals.remove(id);

        0
    })
}

#[pymodule(name = "pybox")]
mod py_pybox {
    use crate::ioctl::{pybox_ioctl_host_req_impl, pybox_ioctl_packet};
    use crate::mem::pybox_free_mem;
    use rustpython_vm::{
        AsObject, PyPayload, PyResult, VirtualMachine,
        builtins::{PyBytes, PyBytesRef, PyDict, PyTuple},
        convert::IntoObject,
        function::FuncArgs,
    };

    /// Python function: pybox_ioctl_host(handle, data) -> (success, result_bytes)
    ///
    /// Host allocates response buffer using pybox_alloc_mem, guest copies data and frees it.
    #[pyfunction]
    fn pybox_ioctl_host(
        handle: isize,
        data: PyBytesRef,
        vm: &VirtualMachine,
    ) -> PyResult<(bool, PyBytesRef)> {
        let data_bytes = data.as_bytes();

        // Prepare request packet
        let mut req = pybox_ioctl_packet {
            buf: data_bytes.as_ptr() as *mut _,
            buf_len: data_bytes.len(),
        };

        // Prepare response packet (host will allocate buffer)
        let mut resp = pybox_ioctl_packet {
            buf: std::ptr::null_mut(),
            buf_len: 0,
        };

        // Call the host ioctl implementation
        #[cfg(target_arch = "wasm32")]
        let success = unsafe {
            pybox_ioctl_host_req_impl(handle as usize, &mut req as *mut _, &mut resp as *mut _) == 0
        };

        #[cfg(not(target_arch = "wasm32"))]
        let success =
            pybox_ioctl_host_req_impl(handle as usize, &mut req as *mut _, &mut resp as *mut _)
                == 0;

        // Create Python bytes object from host-allocated buffer
        let result_bytes = if !resp.buf.is_null() && resp.buf_len > 0 {
            // Copy data from host buffer to Rust Vec
            let data_vec =
                unsafe { std::slice::from_raw_parts(resp.buf as *const u8, resp.buf_len).to_vec() };

            // Free the host-allocated buffer
            pybox_free_mem(resp.buf);

            PyBytes::from(data_vec).into_ref(&vm.ctx)
        } else {
            // Empty response
            PyBytes::from(Vec::new()).into_ref(&vm.ctx)
        };

        Ok((success, result_bytes))
    }

    /// Python function: pybox_json_rpc(handler_id, *args, **kwargs) -> result
    ///
    /// JSON-RPC wrapper around pybox_ioctl_host that handles serialization/deserialization.
    #[pyfunction]
    fn pybox_json_rpc(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // 1. Parse handler_id from first argument
        if args.args.is_empty() {
            return Err(vm.new_type_error(
                "pybox_json_rpc() missing required argument: 'handler_id'".to_string(),
            ));
        }

        let handler_id_obj = &args.args[0];
        let handler_id: isize = handler_id_obj.try_to_value(vm)?;

        // 2. Extract remaining arguments
        let remaining_args = args.args[1..].to_vec();
        let remaining_args_tuple = PyTuple::new_ref(remaining_args, &vm.ctx);

        // 3. Build request dict: {"args": [...], "kwargs": {...}}
        let request_dict = PyDict::default().into_ref(&vm.ctx);
        request_dict
            .as_object()
            .set_item("args", remaining_args_tuple.into_object(), vm)?;

        // Convert kwargs to PyDict
        let kwargs_dict = PyDict::default().into_ref(&vm.ctx);
        for (key, value) in args.kwargs {
            kwargs_dict.as_object().set_item(key.as_str(), value, vm)?;
        }
        request_dict
            .as_object()
            .set_item("kwargs", kwargs_dict.into_object(), vm)?;

        // 4. Serialize to JSON
        let json_module = vm.import("json", 0)?;
        let dumps_func = json_module.get_attr("dumps", vm)?;
        let request_json_str = dumps_func.call((request_dict.to_owned(),), vm)?;

        // 5. Encode to bytes
        let encode_method = request_json_str.get_attr("encode", vm)?;
        let encode_result = encode_method.call((vm.ctx.new_str("utf-8"),), vm)?;
        let request_bytes = encode_result
            .downcast::<PyBytes>()
            .map_err(|_| vm.new_type_error("encode() did not return bytes".to_string()))?;

        // 6. Call pybox_ioctl_host
        let (is_ok, response_data) = pybox_ioctl_host(handler_id, request_bytes, vm)?;

        if !is_ok {
            return Err(vm.new_exception_msg(
                vm.ctx.exceptions.exception_type.to_owned(),
                format!(
                    "JSON-RPC communication failed with handler_id {}!",
                    handler_id
                ),
            ));
        }

        // 7. Decode response
        let decode_method = response_data.as_object().get_attr("decode", vm)?;
        let decode_result = decode_method.call((vm.ctx.new_str("utf-8"),), vm)?;

        // 8. Deserialize JSON
        let loads_func = json_module.get_attr("loads", vm)?;
        let response_dict = loads_func.call((decode_result,), vm)?;

        // 9. Check for exception
        let response_dict_obj = response_dict
            .downcast::<PyDict>()
            .map_err(|_| vm.new_type_error("JSON response is not a dict".to_string()))?;

        if let Ok(exception) = response_dict_obj.get_item("exception", vm) {
            let error_msg = if let Ok(traceback) = response_dict_obj.get_item("traceback", vm) {
                format!(
                    "JSON-RPC Error: {}\nTraceback:\n{}",
                    exception.str(vm)?,
                    traceback.str(vm)?
                )
            } else {
                format!("JSON-RPC Error: {}", exception.str(vm)?)
            };

            return Err(
                vm.new_exception_msg(vm.ctx.exceptions.exception_type.to_owned(), error_msg)
            );
        }

        // 10. Return result
        response_dict_obj.get_item("result", vm).map_err(|_| {
            vm.new_exception_msg(
                vm.ctx.exceptions.exception_type.to_owned(),
                "JSON-RPC response missing 'result' field".to_string(),
            )
        })
    }
}

#[cfg(test)]
mod lib_tests {
    use super::*;

    #[test]
    fn test_pybox_init_local_from() {
        let from_id = pybox_bytes::new_bytes(b"source_local");
        let result = pybox_init_local(from_id);
        assert_eq!(result, 0, "Failed to create source local");

        let new_id = pybox_bytes::new_bytes(b"copied_local");
        let result = pybox_init_local_from(new_id, from_id);
        assert_eq!(result, 0, "Failed to copy local");

        let result = pybox_init_local_from(new_id, from_id);
        assert_eq!(result, -1, "Should fail when target already exists");

        let nonexistent = pybox_bytes::new_bytes(b"nonexistent");
        let another_id = pybox_bytes::new_bytes(b"another_local");
        let result = pybox_init_local_from(another_id, nonexistent);
        assert_eq!(result, -1, "Should fail when source doesn't exist");
    }
}
