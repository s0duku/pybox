#![allow(dead_code)]

use crate::reactor::PyBoxReactor;
use pyo3::prelude::*;

/// 简单的内存快照
/// 用法：
///   snapshot = PyBoxReactorSnapshot(reactor)  # 保存当前状态
///   # ... 执行一些操作
///   snapshot.restore(reactor)  # 恢复到快照时刻
#[pyclass(subclass)]
pub struct PyBoxReactorSnapshot {
    /// 保存的内存快照
    snapshot: Option<Vec<u8>>,
}
/// 不适用 COW 等方式的情况，很难避免全量扫描，不如直接拷贝存储
#[pymethods]
impl PyBoxReactorSnapshot {
    #[new]
    #[pyo3(signature = (*_args, **_kwargs))]
    fn new(
        _args: &Bound<'_, pyo3::types::PyTuple>,
        _kwargs: Option<&Bound<'_, pyo3::types::PyDict>>,
    ) -> Self {
        Self { snapshot: None }
    }

    /// 初始化快照，保存当前内存状态
    fn __init__(&mut self, reactor: &PyBoxReactor) -> pyo3::PyResult<()> {
        reactor.safe_access(|| {
            let Some(core) = reactor.core.as_ref() else {
                return Err(pyo3::exceptions::PyRuntimeError::new_err(
                    "Can not fetch PyBoxReactorCore!",
                ));
            };

            let store_ptr = reactor
                .store
                .as_ref()
                .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Store not initialized"))?
                .get();
            let store = unsafe { &*store_ptr };

            let Some(memory) = core.get_memory() else {
                return Err(pyo3::exceptions::PyRuntimeError::new_err(
                    "Can not get PyBoxReactor Memory!",
                ));
            };

            // 保存完整内存快照
            let data = memory.data(store);
            self.snapshot = Some(data.to_vec());
            Ok(())
        })
    }

    /// 恢复到快照时的内存状态
    fn restore(&self, reactor: &PyBoxReactor) -> pyo3::PyResult<()> {
        reactor.safe_access(|| {
            let Some(core) = reactor.core.as_ref() else {
                return Err(pyo3::exceptions::PyRuntimeError::new_err(
                    "Can not fetch PyBoxReactorCore!",
                ));
            };

            let store_ptr = reactor
                .store
                .as_ref()
                .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Store not initialized"))?
                .get();
            let store = unsafe { &mut *store_ptr };

            let Some(memory) = core.get_memory() else {
                return Err(pyo3::exceptions::PyRuntimeError::new_err(
                    "Can not get PyBoxReactor Memory!",
                ));
            };

            let Some(snapshot) = &self.snapshot else {
                return Err(pyo3::exceptions::PyRuntimeError::new_err(
                    "No snapshot available! Call __init__ first.",
                ));
            };

            // 恢复内存
            let memory_data = memory.data_mut(store);
            let copy_len = std::cmp::min(memory_data.len(), snapshot.len());
            memory_data[..copy_len].copy_from_slice(&snapshot[..copy_len]);

            Ok(())
        })
    }

    /// 更新快照为当前状态（可选功能）
    fn update(&mut self, reactor: &PyBoxReactor) -> pyo3::PyResult<()> {
        self.__init__(reactor)
    }

    /// 获取快照大小（字节数）
    fn size(&self) -> usize {
        self.snapshot.as_ref().map(|s| s.len()).unwrap_or(0)
    }
}
