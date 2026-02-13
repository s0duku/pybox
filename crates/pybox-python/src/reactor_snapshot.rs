#![allow(dead_code)]



use crate::reactor::PyBoxReactor;
use pyo3::prelude::*;


#[pyclass]
pub struct PyBoxReactorSnapshotCheckpoint {

}



#[pyclass]
pub struct PyBoxReactorSnapshot {
    base:Option<std::vec::Vec<u8>>,
}


#[pymethods]
impl PyBoxReactorSnapshot {
    #[new]
    #[pyo3(signature = (*_args, **_kwargs))]
    fn new(_args: &Bound<'_, pyo3::types::PyTuple>, _kwargs: Option<&Bound<'_, pyo3::types::PyDict>>) -> Self {
        Self {
            base:None
        }
    }

    fn __init__(
        &mut self,
        reactor:&PyBoxReactor
    ) -> pyo3::PyResult<()> {

        reactor.safe_access(||
            {
                let Some(core) = reactor.core.as_ref() else {
                    return Err(pyo3::exceptions::PyRuntimeError::new_err("Can not fetch PyBoxReactorCore!"))
                };

                let store_ptr = reactor.store.as_ref()
                    .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Store not initialized"))?
                    .get();
                let store = unsafe { & *store_ptr };

                let Some(memory) = core.get_memory() else {
                    return Err(pyo3::exceptions::PyRuntimeError::new_err("Can not get PyBoxReactor Memory!"))
                };
                let data = memory.data(store);
                self.base = Some(data.to_vec());             
                Ok(())
            }
        )
    }
}