mod reactor;
mod reactor_snapshot;

use pyo3::prelude::*;

/// A Python module implemented in Rust.
#[pymodule]
fn pyboxcore(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<reactor::PyBoxReactor>()?;
    m.add_class::<reactor::PyBoxReactorCore>()?;
    m.add_class::<reactor_snapshot::PyBoxReactorSnapshot>()?;
    Ok(())
}
