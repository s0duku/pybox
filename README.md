<div align="center">
  <h1>PyBox</h1>
</div>
<div align="center">
  <h3>In-process sandboxed-python based on RustPython and WASM Reactor mode for building LLM Agent's PTC (Programmatic Tool Calling) style tools</h3>
</div>
<div align="center">

![CI](https://github.com/s0duku/pybox/actions/workflows/ci.yml/badge.svg)
![License](https://img.shields.io/badge/license-MIT-blue.svg)

</div>

---
**NOTE:** This project is still in development, lots of features and designs may not be stable

PyBox is a **experimental** sandboxed-python based on RustPython and WASM Reactor mode

What **benefits** can you get from PyBox?
* Implement PTC (Programmatic Tool Calling) style tools from Anthropic for your Agent
* Locally and **cross-platform** sandboxed-python enviroment based on WASM32-WASI
* The complete Python implementation from RustPython
* One WASM instance but multiple execution context
* **Persistent execution context**, like a **Python REPL**
* **Variable protection**, protecting any variable from being accidentally modified by the agent
* Object transfer, transfer **JSON-serializable** object to the execution context.
* Inherited execution context to create **hierarchical context** relationship
* In-process sandbox, consistent thread context with the host caller and reentrant code execution(thanks for WASM Reactor mode frame management)
* Enjoy all the features of WASM runtime: `directory mapping`, `FFI`, `snapshot`, `fuel`, `compiled-cache` ...etc
* You can achieve a similar implementation using CPython in conjunction with the WASI-SDK. However, we believe that Rust would be more accessible when integrated with WASM.

**disadvantages**
* There is a certain performance loss with WASM
* Consistency with the calling thread brings advantages in terms of synchronization logic, but at the same time, the multi-threading and asynchronous support of WASM have limitations. When you need to stop after a timeout, you may need to use `fuel` and `snapshot` to achieve it
* **It is recommended to create separate instances for each thread**
* Although the WASM runtime can handle exceptions in WASM, at the language level, it is still possible to result in incomplete cleanup. Therefore, the most reliable approach is still to use `snapshot`.
* Can not support native-python(CPython module) package due to WASI compatibility(WASMER's WASIX has part of support)

---

## Install
```bash
# for use
pip install .

# for devel
pip install -e .

# optional, build pybox.wasm, need rust enviroment(`rustup target add wasm32-wasip1`)
python build_wasm.py

```


## Usage

See more codes under `examples/`

Simple usage, persistent context like a REPL

```python
from pybox.box import PyBox

box = PyBox()
id = "test_exec"
assert box.init_local(id) # create execution context

code = """
import sys
print(sys.modules)
test_var = 1
"""

print(box.exec(code,id))

code = """
print(test_var)
"""

print(box.exec(code,id))
```

Define tool function and protect your stub inside sandbox

```python

@box.tool
def hello_host(name):
    return f"Hello {name}"

box.exec(hello_host.stub(),id)
box.protect(id,hello_host.name)

code = """
print(hello_host('pybox'))
hello_host = "try modify"
"""
print(box.exec(code,id))

```

You can also utilize thread consistency with the host to construct LLM reasoning contexts that are capable of recursion and automatic cleanup, and which can be executed in conjunction with the code!


## Alternatives

There are many sandboxed implementations of Python. Below are some simple tests conducted in my local environment for reference only.

| Tech    | Initialize latency | Execution Time | Language completeness | Security | File mounting | Snapshotting |
|---------|--------------------|----------------|----------------------|----------| -------- | -------- |
| Monty   |0.057ms             |0.033ms         | partial   |   strict | easy | easy |
| PyBox   |<ul><li>First load: 200.926ms</li><li>Instance creation: 14.319ms</li><li>Context init: 7.911ms</li></ul> |0.066ms         | almost | strict | easy | working on |
| Pyodide |4567.486ms (mcp-run-python)          |2135.502ms | full | poor | easy | hard |
| Docker  |525.731ms           |/               | full | good | easy | intermediate |

See `scripts/benchmarks.py` to get details.




---

## Contributing

**PyBox is an experimental project exploring WASM-based Python sandboxing.** If you resonate with this approach—favoring in-process execution, thread consistency, and WASM's security primitives over traditional isolation methods—**you're welcome to contribute.**

This project tackles interesting challenges: **safe code execution, variable protection, context isolation, and snapshot-based recovery**. We're building a foundation for WASM-based agent sandboxing, and we believe the community can help push these ideas further.

**If you believe in this direction, contributions are welcome.** Code, bug reports, design feedback—all help shape what PyBox can become.

---

## License

PyBox is released under the [MIT License](LICENSE). See the LICENSE file for details.