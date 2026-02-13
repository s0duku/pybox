//! exec crate 提供 pybox_exec 等在 locals 中执行代码的接口

use std::rc::Rc;

use libc::ssize_t;

use rustpython_vm::{AsObject, Interpreter, PyResult, VirtualMachine, compiler::Mode};

use super::PYBOX_STATE;

use crate::ioctl;
use crate::protected::ProtectedLocals;

/// 在指定 id 的 locals 环境上创建一个 json 描述的变量
///
/// # Arguments
///
/// * `id` 指定 locals 环境 id
/// * `name` 变量名
/// * `object` 序列化的 json 对象
#[unsafe(no_mangle)]
pub extern "C" fn pybox_assign(
    id: *const ioctl::pybox_bytes,
    name: *const ioctl::pybox_bytes,
    object: *const ioctl::pybox_bytes,
    error: *mut *mut ioctl::pybox_bytes,
) -> ssize_t {
    unsafe {
        if id.is_null() || name.is_null() || object.is_null() {
            if !error.is_null() {
                *error =
                    ioctl::pybox_bytes::new_bytes(b"Invalid arguments: id, name or object is null");
            }
            return -1;
        }
    }

    PYBOX_STATE.with_borrow(|pybox_state| {
        let Ok((id, name, object_str)) = (|| -> Result<_, ()> {
            unsafe {
                let id: &str = (*id).string()?;
                let name = (*name).string()?;
                let object_str = (*object).string()?;
                Ok((id, name, object_str))
            }
        })() else {
            if !error.is_null() {
                unsafe {
                    *error = ioctl::pybox_bytes::new_bytes(
                        b"Invalid UTF-8 encoding in id, name or object",
                    );
                }
            }
            return -1;
        };

        let Some((locals, interpreter)) = pybox_state.locals.get(id) else {
            let error_msg = format!("Local context '{}' not found", id);
            if !error.is_null() {
                unsafe {
                    *error = ioctl::pybox_bytes::new_bytes(error_msg.as_bytes());
                }
            }
            return -1;
        };

        // Use JSON to deserialize object string to Python object and save to locals
        interpreter.enter(|vm| {
            let result = (|| -> PyResult<()> {
                // Import json module
                let json_module = vm.import("json", 0)?;
                let loads_func = json_module.get_attr("loads", vm)?;

                // Deserialize JSON string to Python object
                let json_str = vm.ctx.new_str(object_str);
                let python_obj = loads_func.call((json_str,), vm)?;

                // Get the ProtectedLocals instance
                let protected_locals =
                    locals.downcast_ref::<ProtectedLocals>().ok_or_else(|| {
                        vm.new_type_error("locals is not a ProtectedLocals instance".to_string())
                    })?;

                // Directly set item to internal dict, bypassing protection check
                let dict = protected_locals.dict();
                dict.as_object().set_item(name, python_obj, vm)?;

                Ok(())
            })();

            match result {
                Ok(_) => 0,
                Err(exception) => {
                    // Write error to error_buf if provided
                    let mut error_string = String::new();
                    match vm.write_exception(&mut error_string, &exception) {
                        Ok(_) => (),
                        Err(_) => {
                            error_string.push_str("Failed to assign object: unknown error");
                        }
                    }
                    if !error.is_null() {
                        unsafe {
                            *error = ioctl::pybox_bytes::new_bytes(error_string.as_bytes());
                        }
                    }
                    -1
                }
            }
        })
    })
}

/// redirect rustpython vm stdout/stderr to string
/// * `vm` rustpython vm
/// * `output` string buffer
/// * `f` code run in vm
pub fn with_redirect_output<F, R>(vm: &VirtualMachine, output: &mut String, f: F) -> R
where
    F: FnOnce() -> R,
{
    let Ok(output_capture) = (|| -> PyResult<_> {
        // Use _io.StringIO directly instead of io.StringIO to avoid FileIO dependency
        let io_module = vm.import("_io", 0)?;
        let string_io_class = io_module.get_attr("StringIO", vm)?;
        let output_capture = string_io_class.call((), vm)?;
        Ok(output_capture)
    })() else {
        return f();
    };

    let Ok((sys_module, original_stdout, original_stderr)) = (|| -> PyResult<_> {
        let sys_module = vm.import("sys", 0)?;
        let original_stdout = sys_module.get_attr("stdout", vm)?;
        let original_stderr = sys_module.get_attr("stderr", vm)?;
        Ok((sys_module, original_stdout, original_stderr))
    })() else {
        return f();
    };

    // set redirect
    let _ = sys_module.set_attr("stdout", output_capture.clone(), vm);
    let _ = sys_module.set_attr("stderr", output_capture.clone(), vm);

    let result = f();

    // recover stdout/stderr
    let _ = sys_module.set_attr("stdout", original_stdout, vm);
    let _ = sys_module.set_attr("stderr", original_stderr, vm);

    // store to string
    if let Ok(output_content) = vm.call_method(&output_capture, "getvalue", ()) {
        if let Ok(output_str) = output_content.try_into_value::<String>(vm) {
            output.push_str(&output_str);
        }
    }

    result
}

/// 在指定 locals 环境中执行 python 代码
/// * `id` 指定 locals id
/// * `code` python 代码
/// * `output_buf` 执行输出 (stdout & stderr)
/// * `error_buf` pybox 错误信息
#[unsafe(no_mangle)]
pub extern "C" fn pybox_exec(
    id: *const ioctl::pybox_bytes,
    code: *const ioctl::pybox_bytes,
    output: *mut *mut ioctl::pybox_bytes,
    error: *mut *mut ioctl::pybox_bytes,
) -> ssize_t {
    if id.is_null() || code.is_null() {
        if !error.is_null() {
            unsafe {
                *error = ioctl::pybox_bytes::new_bytes(b"Invalid arguments: id or code is null");
            }
        }
        return -1;
    }

    // Parse id and code
    let Ok((id, code)) = (|| -> Result<_, ()> {
        unsafe {
            let id = (*id).string()?;
            let code = (*code).string()?;
            Ok((id, code))
        }
    })() else {
        if !error.is_null() {
            unsafe {
                *error = ioctl::pybox_bytes::new_bytes(b"Invalid UTF-8 encoding in id or code");
            }
        }
        return -1;
    };

    // Step 1: Get interpreter and locals (with read-only borrow)
    // Clone them so we can release the borrow before executing Python code
    let (interpreter, locals_ref) = match PYBOX_STATE.with_borrow(
        |pybox_state| -> Result<(Rc<Interpreter>, rustpython_vm::PyObjectRef), &'static str> {
            let Some((locals, interpreter)) = pybox_state.locals.get(id) else {
                return Err("Local context not found");
            };

            // Clone Rc<Interpreter> and PyObjectRef (cheap, reference-counted)
            Ok((interpreter.clone(), locals.clone()))
        },
    ) {
        Ok(values) => values,
        Err(err_msg) => {
            if !error.is_null() {
                unsafe {
                    *error = ioctl::pybox_bytes::new_bytes(err_msg.as_bytes());
                }
            }
            return -1;
        }
    };

    // Step 2: Execute code WITHOUT holding PYBOX_STATE lock
    // This allows Python code to call pybox functions (like init_local_from) via JSON-RPC
    interpreter.enter(|vm| {
        let mut output_string = String::new();

        let code_obj = match vm.compile(&code, Mode::Exec, "<string>".to_owned()) {
            Ok(code_obj) => code_obj,
            Err(err) => {
                // 处理编译错误
                let exception = vm.new_syntax_error(&err, Some(&code));
                match vm.write_exception(&mut output_string, &exception) {
                    Ok(_) => (),
                    Err(_) => {
                        output_string.push_str("Pybox: Compile Code Failed!");
                    }
                }
                if !output.is_null() {
                    unsafe {
                        *output = ioctl::pybox_bytes::new_bytes(output_string.as_bytes());
                    }
                }
                return 0;
            }
        };

        // 将 locals PyObjectRef 转换为 ProtectedLocals
        let protected_locals = locals_ref
            .clone()
            .downcast::<ProtectedLocals>()
            .expect("locals must be ProtectedLocals");

        // 使用 ProtectedLocals 作为 locals，内部 dict 作为 globals
        let scope = rustpython_vm::scope::Scope::with_builtins(
            Some(rustpython_vm::function::ArgMapping::new(locals_ref)),
            protected_locals.dict().to_owned(), // 使用内部字典作为 globals
            vm,
        );

        match with_redirect_output(vm, &mut output_string, || vm.run_code_obj(code_obj, scope)) {
            Ok(_) => (),
            Err(exception) => {
                match vm.write_exception(&mut output_string, &exception) {
                    Ok(_) => (),
                    Err(_) => {
                        output_string.push_str("Pybox: Run Code Failed!");
                    }
                };
            }
        };

        // write output to buffer
        if !output.is_null() {
            unsafe {
                *output = ioctl::pybox_bytes::new_bytes(output_string.as_bytes());
            }
        }
        0
    })
}

#[cfg(test)]
mod test {
    use crate::ioctl;
    use crate::mem::pybox_alloc_mem;
    use crate::protected::pybox_local_protect;
    use crate::pybox_init_local;

    use super::*;

    #[test]
    fn test_io_module_contents() {
        let id = ioctl::pybox_bytes::new_bytes(b"test_io_check");
        let result = pybox_init_local(id);
        assert_eq!(result, 0);

        let code = ioctl::pybox_bytes::new_bytes(
            r#"
import _io
print("_io module contents:")
print(dir(_io))
print("\nChecking for FileIO:")
print(hasattr(_io, 'FileIO'))
"#
            .as_bytes(),
        );

        let output_buf = pybox_alloc_mem(std::mem::size_of::<*mut ioctl::pybox_bytes>());
        let result = pybox_exec(
            id,
            code,
            output_buf as *mut *mut ioctl::pybox_bytes,
            std::ptr::null_mut(),
        );

        assert_eq!(result, 0);
        unsafe {
            println!(
                "{}",
                (*(*(output_buf as *mut *mut ioctl::pybox_bytes)))
                    .string()
                    .unwrap()
            );
        }
    }

    #[test]
    fn test_pybox_exec() {
        let id = ioctl::pybox_bytes::new_bytes(b"test_pybox_exec");
        let result = pybox_init_local(id);
        assert_eq!(result, 0);

        let name = ioctl::pybox_bytes::new_bytes(b"my_var");
        let result = pybox_local_protect(id, name);
        assert_eq!(result, 0);

        let code = ioctl::pybox_bytes::new_bytes(
            r#"
import pybox

print(pybox)

def test():
    print(pybox_ioctl_host)
    my_var = 20
    print(pybox_json_rpc)

test()

test_continue = "continued!"

# 通过赋值操作（应该触发 __setitem__）
print("Test 1: Assignment to my_var")
my_var = 10
print(f"After assignment, my_var = {my_var}")
                        "#
            .as_bytes(),
        );

        let output_buf = pybox_alloc_mem(std::mem::size_of::<*mut ioctl::pybox_bytes>());

        let result = pybox_exec(
            id,
            code,
            output_buf as *mut *mut ioctl::pybox_bytes,
            std::ptr::null_mut(),
        );

        println!("\n=================================================================");
        assert_eq!(result, 0, "execution test failed!");
        unsafe {
            println!(
                "{}",
                (*(*(output_buf as *mut *mut ioctl::pybox_bytes)))
                    .string()
                    .unwrap()
            );
        }

        let code = ioctl::pybox_bytes::new_bytes(
            r#"
print(test_continue)
                        "#
            .as_bytes(),
        );

        let result = pybox_exec(
            id,
            code,
            output_buf as *mut *mut ioctl::pybox_bytes,
            std::ptr::null_mut(),
        );

        println!("\n=================================================================");
        assert_eq!(result, 0, "execution test failed!");

        unsafe {
            println!(
                "{}",
                (*(*(output_buf as *mut *mut ioctl::pybox_bytes)))
                    .string()
                    .unwrap()
            );
        }
    }

    #[test]
    fn test_pybox_assign() {
        let id = ioctl::pybox_bytes::new_bytes(b"test_pybox_assign");
        let result = pybox_init_local(id);
        assert_eq!(result, 0, "Failed to init local");

        // Protect a variable
        let protected_var = ioctl::pybox_bytes::new_bytes(b"protected_var");
        let result = pybox_local_protect(id, protected_var);
        assert_eq!(result, 0, "Failed to protect variable");

        // Test 1: Assign a simple value to a non-protected variable
        let var_name = ioctl::pybox_bytes::new_bytes(b"test_var");
        let json_value = ioctl::pybox_bytes::new_bytes(b"42");

        let result = pybox_assign(id, var_name, json_value, std::ptr::null_mut());
        assert_eq!(result, 0, "Failed to assign simple value");

        // Test 2: Assign to protected variable (should succeed since pybox_assign bypasses protection)
        let json_value2 = ioctl::pybox_bytes::new_bytes(b"\"bypassed!\"");
        let result = pybox_assign(id, protected_var, json_value2, std::ptr::null_mut());
        assert_eq!(
            result, 0,
            "Failed to assign to protected variable (should bypass protection)"
        );

        // Test 3: Assign a complex object
        let complex_var = ioctl::pybox_bytes::new_bytes(b"complex_obj");
        let json_complex = ioctl::pybox_bytes::new_bytes(
            br#"{"key": "value", "number": 123, "array": [1, 2, 3]}"#,
        );
        let result = pybox_assign(id, complex_var, json_complex, std::ptr::null_mut());
        assert_eq!(result, 0, "Failed to assign complex object");

        // Test 4: Verify the assignments by executing code
        let code = ioctl::pybox_bytes::new_bytes(
            r#"
print(f"test_var = {test_var}")
print(f"protected_var = {protected_var}")
print(f"complex_obj = {complex_obj}")
"#
            .as_bytes(),
        );

        let output_buf = pybox_alloc_mem(std::mem::size_of::<*mut ioctl::pybox_bytes>());
        let result = pybox_exec(
            id,
            code,
            output_buf as *mut *mut ioctl::pybox_bytes,
            std::ptr::null_mut(),
        );

        println!("\n=================================================================");
        println!("Test pybox_assign output:");
        assert_eq!(result, 0, "Execution failed");
        unsafe {
            let output = (*(*(output_buf as *mut *mut ioctl::pybox_bytes)))
                .string()
                .unwrap();
            println!("{}", output);
            assert!(output.contains("test_var = 42"), "test_var should be 42");
            assert!(
                output.contains("protected_var = bypassed!"),
                "protected_var should be bypassed!"
            );
            assert!(
                output.contains("complex_obj = "),
                "complex_obj should exist"
            );
        }
    }
}
