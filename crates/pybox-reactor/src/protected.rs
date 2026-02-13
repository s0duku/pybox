//! protected.rs for supporting protected locals

use libc::ssize_t;

use rustpython_vm::{
    AsObject, Py, PyObject, PyObjectRef, PyResult, VirtualMachine,
    builtins::{PyDict, PyDictRef, PyStr, PyType},
    common::lock::PyRwLock,
    function::FuncArgs,
    object::{PyPayload, Traverse, TraverseFn},
    protocol::PyMappingMethods,
    pyclass,
    types::{AsMapping, Constructor},
};
use std::collections::HashSet;

/// ProtectedLocals: 带保护键的字典
/// 使用组合模式包装 PyDict，实现 AsMapping trait 来拦截操作
#[pyclass(
    name = "ProtectedLocals",
    module = false,
    unhashable = true,
    traverse = "manual"
)]
#[derive(Debug, rustpython_vm::PyPayload)]
pub struct ProtectedLocals {
    dict: PyDictRef,                          // 内部字典
    protected_set: PyRwLock<HashSet<String>>, // 受保护的键集合（不需要遍历）
}

// SAFETY: Traverse properly visits all owned PyObjectRefs
unsafe impl Traverse for ProtectedLocals {
    fn traverse(&self, traverse_fn: &mut TraverseFn<'_>) {
        self.dict.traverse(traverse_fn);
    }
}

impl Constructor for ProtectedLocals {
    type Args = FuncArgs;

    fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
        // 创建内部字典
        let dict = if args.args.is_empty() && args.kwargs.is_empty() {
            PyDict::default()
        } else {
            PyDict::py_new(_cls, args, vm)?
        };

        Ok(Self {
            dict: dict.into_ref(&vm.ctx),
            protected_set: PyRwLock::new(HashSet::new()),
        })
    }
}

// Rust API - 不暴露给 Python
impl ProtectedLocals {
    /// 获取内部字典引用
    pub fn dict(&self) -> &PyDictRef {
        &self.dict
    }

    /// 保护某个键，使其不可被修改或删除
    pub fn protect(&self, key: &str) {
        self.protected_set.write().insert(key.to_owned());
    }

    /// 取消保护某个键
    #[allow(unused)]
    pub fn unprotect(&self, key: &str) {
        self.protected_set.write().remove(key);
    }

    /// 检查键是否被保护
    #[allow(unused)]
    pub fn is_protected(&self, key: &str) -> bool {
        self.protected_set.read().contains(key)
    }

    /// 获取所有被保护的键列表
    #[allow(unused)]
    pub fn get_protected_keys(&self) -> Vec<String> {
        self.protected_set.read().iter().cloned().collect()
    }

    /// 检查键是否被保护（从 PyObject 转换）
    fn check_protected(&self, key: &PyObject, _vm: &VirtualMachine) -> PyResult<bool> {
        if let Some(key_str) = key.downcast_ref::<PyStr>() {
            Ok(self.protected_set.read().contains(key_str.as_str()))
        } else {
            Ok(false)
        }
    }
}

// 实现 AsMapping trait 技能自定义映射类型
impl AsMapping for ProtectedLocals {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: PyMappingMethods = PyMappingMethods {
            length: Some(|mapping, _vm| {
                let zelf = ProtectedLocals::mapping_downcast(mapping);
                // 返回内部字典的长度
                Ok(zelf.dict.__len__())
            }),

            subscript: Some(|mapping, needle, vm| {
                let zelf = ProtectedLocals::mapping_downcast(mapping);
                // 直接从内部字典获取
                zelf.dict.as_object().get_item(needle, vm)
            }),

            ass_subscript: Some(|mapping, needle, value, vm| {
                let zelf = ProtectedLocals::mapping_downcast(mapping);

                if let Some(value) = value {
                    // 设置操作 - 检查是否被保护
                    if zelf.check_protected(needle, vm)? {
                        if let Some(key_str) = needle.downcast_ref::<PyStr>() {
                            return Err(vm.new_key_error(
                                vm.ctx
                                    .new_str(format!(
                                        "Cannot modify protected key: '{}'",
                                        key_str.as_str()
                                    ))
                                    .into(),
                            ));
                        }
                    }
                    // 未保护，允许设置
                    zelf.dict.as_object().set_item(needle, value, vm)
                } else {
                    // 删除操作 - 检查是否被保护
                    if zelf.check_protected(needle, vm)? {
                        if let Some(key_str) = needle.downcast_ref::<PyStr>() {
                            return Err(vm.new_key_error(
                                vm.ctx
                                    .new_str(format!(
                                        "Cannot delete protected key: '{}'",
                                        key_str.as_str()
                                    ))
                                    .into(),
                            ));
                        }
                    }
                    // 未保护，允许删除
                    zelf.dict.as_object().del_item(needle, vm)
                }
            }),
        };
        &AS_MAPPING
    }
}

#[pyclass(with(Constructor, AsMapping))]
impl ProtectedLocals {
    /// Python 接口：获取长度
    #[pymethod(name = "__len__")]
    fn len(&self) -> usize {
        self.dict.__len__()
    }

    /// Python 接口：获取项
    #[pymethod(name = "__getitem__")]
    fn getitem(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.dict.as_object().get_item(&*key, vm)
    }

    /// Python 接口：设置项（会调用 AsMapping 的 ass_subscript）
    #[pymethod(name = "__setitem__")]
    fn setitem(&self, key: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // 检查保护
        if self.check_protected(&*key, vm)? {
            if let Some(key_str) = key.downcast_ref::<PyStr>() {
                return Err(vm.new_key_error(
                    vm.ctx
                        .new_str(format!(
                            "Cannot modify protected key: '{}'",
                            key_str.as_str()
                        ))
                        .into(),
                ));
            }
        }
        self.dict.as_object().set_item(&*key, value, vm)
    }

    /// Python 接口：删除项
    #[pymethod(name = "__delitem__")]
    fn delitem(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // 检查保护
        if self.check_protected(&*key, vm)? {
            if let Some(key_str) = key.downcast_ref::<PyStr>() {
                return Err(vm.new_key_error(
                    vm.ctx
                        .new_str(format!(
                            "Cannot delete protected key: '{}'",
                            key_str.as_str()
                        ))
                        .into(),
                ));
            }
        }
        self.dict.as_object().del_item(&*key, vm)
    }

    /// Python 接口：迭代键
    #[pymethod(name = "__iter__")]
    fn iter(&self, vm: &VirtualMachine) -> PyResult {
        vm.call_method(self.dict.as_object(), "__iter__", ())
    }

    /// Python 接口：字符串表示（显示为普通字典）
    #[pymethod(name = "__repr__")]
    fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
        // 直接返回内部字典的 repr，看起来像普通字典
        let dict_repr = self.dict.as_object().repr(vm)?;
        Ok(dict_repr.as_str().to_string())
    }

    /// Python 接口：字符串表示（print 使用）
    #[pymethod(name = "__str__")]
    fn str(&self, vm: &VirtualMachine) -> PyResult<String> {
        // 和 __repr__ 一样，显示为普通字典
        self.repr(vm)
    }

    /// Python 接口：支持 dir() 方法
    #[pymethod(name = "__dir__")]
    fn dir(&self, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        // 直接获取内部字典的迭代器（迭代 keys）
        let keys_iter = self.dict.as_object().get_iter(vm)?;
        let mut attrs = Vec::new();

        loop {
            match keys_iter.next(vm)? {
                rustpython_vm::protocol::PyIterReturn::Return(key) => {
                    attrs.push(key);
                }
                rustpython_vm::protocol::PyIterReturn::StopIteration(_) => break,
            }
        }

        Ok(attrs)
    }

    /// Python 接口：keys() 方法（dir() 内部需要）
    #[pymethod]
    fn keys(&self, vm: &VirtualMachine) -> PyResult {
        vm.call_method(self.dict.as_object(), "keys", ())
    }
}

use crate::{PYBOX_STATE, ioctl};

#[unsafe(no_mangle)]
pub extern "C" fn pybox_local_protect(
    id: *const ioctl::pybox_bytes,
    name: *const ioctl::pybox_bytes,
) -> ssize_t {
    PYBOX_STATE.with_borrow_mut(|pybox_state| {
        let Ok((id, name)) = (|| -> Result<_, ()> {
            unsafe {
                let id = (*id).string()?;
                let name = (*name).string()?;
                Ok((id, name))
            }
        })() else {
            return -1;
        };

        let Some(locals) = pybox_state.locals.get(id) else {
            return -1;
        };

        let locals = locals
            .0
            .downcast_ref::<ProtectedLocals>()
            .expect("unable to convert ProtectedLocals!");

        locals.protect(&name);
        0
    })
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::*;

    #[test]
    pub fn test_protected_dict() {
        let id = ioctl::pybox_bytes::new_bytes(b"test_pybox_exec");
        let name = ioctl::pybox_bytes::new_bytes(b"my_var");
        let result = pybox_init_local(id);
        assert_eq!(result, 0);
        let result = pybox_local_protect(id, name);
        assert_eq!(result, 0);
    }
}
