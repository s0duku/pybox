#![allow(dead_code)]

use std::{collections::HashMap, mem::size_of, sync::Arc};

// WASI Preview 1
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi::preview1::WasiP1Ctx;

// WASM 类型别名，增强代码可读性
/// WASM 内存中的 32 位指针/地址
type WasmPtr = u32;
/// WASM 内存中的大小值
type WasmSize = u32;
/// WASM ioctl handle ID
type HandleId = u32;


/// WASM 端的 pybox_bytes 结构（仅用于文档）
#[allow(dead_code)]
#[repr(C, packed)]
pub struct PyboxBytes {
    pub length: WasmSize,
    pub data: [u8; 0]
}

/// WASM 端的 ioctl packet 结构
/// C 结构: struct pybox_ioctl_packet { void* buf; size_t buf_len; }
#[repr(C, packed)]
struct IoctlPacket {
    buf: WasmPtr,
    buf_len: WasmSize,
}

impl IoctlPacket {
    /// 从 WASM 内存中读取 IoctlPacket
    fn read_from_memory(
        memory: &wasmtime::Memory,
        caller: &wasmtime::Caller<'_, WasiP1Ctx>,
        ptr: WasmPtr,
    ) -> Result<Self, String> {
        let memory_data = memory.data(caller);
        let ptr_usize = ptr as usize;
        let packet_size = size_of::<Self>();

        if ptr_usize + packet_size > memory_data.len() {
            return Err("Packet pointer out of bounds".to_string());
        }

        let packet_bytes = &memory_data[ptr_usize..ptr_usize + packet_size];

        Ok(Self {
            buf: WasmPtr::from_le_bytes([
                packet_bytes[0],
                packet_bytes[1],
                packet_bytes[2],
                packet_bytes[3],
            ]),
            buf_len: WasmSize::from_le_bytes([
                packet_bytes[4],
                packet_bytes[5],
                packet_bytes[6],
                packet_bytes[7],
            ]),
        })
    }

    /// 写入 IoctlPacket 到 WASM 内存
    fn write_to_memory(
        &self,
        memory: &wasmtime::Memory,
        caller: &mut wasmtime::Caller<'_, WasiP1Ctx>,
        ptr: WasmPtr,
    ) -> Result<(), String> {
        let memory_data = memory.data_mut(caller);
        let ptr_usize = ptr as usize;
        let packet_size = size_of::<Self>();

        if ptr_usize + packet_size > memory_data.len() {
            return Err("Packet pointer out of bounds".to_string());
        }

        memory_data[ptr_usize..ptr_usize + 4].copy_from_slice(&self.buf.to_le_bytes());
        memory_data[ptr_usize + 4..ptr_usize + 8].copy_from_slice(&self.buf_len.to_le_bytes());

        Ok(())
    }
}

// 模块缓存的 Key：Engine + 文件路径
// Engine 不实现 Hash，所以我们通过指针地址来实现
#[derive(Clone)]
pub struct ModuleCacheKey {
    engine: Arc<wasmtime::Engine>,
    filepath: String,
}

// 手动实现 Hash：使用 Engine 的指针地址
impl std::hash::Hash for ModuleCacheKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // 使用 Engine 的指针地址
        let ptr = Arc::as_ptr(&self.engine) as usize;
        ptr.hash(state);
        self.filepath.hash(state);
    }
}

// 手动实现 PartialEq：比较指针地址和路径
impl PartialEq for ModuleCacheKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.engine, &other.engine) && self.filepath == other.filepath
    }
}

// 手动实现 Eq
impl Eq for ModuleCacheKey {}

impl ModuleCacheKey {
    pub fn new(engine: Arc<wasmtime::Engine>, filepath: String) -> Self {
        Self { engine, filepath }
    }
}

static MODULE_CACHES: std::sync::LazyLock<dashmap::DashMap<ModuleCacheKey, Arc<wasmtime::Module>>> =
    std::sync::LazyLock::new(|| dashmap::DashMap::new());

static DEFAULT_ENGINE: std::sync::LazyLock<Arc<wasmtime::Engine>> =
    std::sync::LazyLock::new(|| {
        let mut config = wasmtime::Config::new();
        // 启用编译缓存
        config.cache_config_load_default().unwrap();
        Arc::new(wasmtime::Engine::new(&config).unwrap())
    });

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyBytesMethods};


#[pyclass]
#[derive(Default)]
pub struct PyBoxReactorCore {
    handlers: dashmap::DashMap<HandleId, Py<PyAny>>,
    alloc_mem: std::sync::OnceLock<wasmtime::TypedFunc<WasmSize, WasmPtr>>,
    free_mem: std::sync::OnceLock<wasmtime::TypedFunc<WasmPtr, ()>>,
    init_local: std::sync::OnceLock<wasmtime::TypedFunc<WasmPtr, i32>>,
    init_local_from:std::sync::OnceLock<wasmtime::TypedFunc<(WasmPtr, WasmPtr), i32>>,
    del_local: std::sync::OnceLock<wasmtime::TypedFunc<WasmPtr, i32>>,
    assign:std::sync::OnceLock<wasmtime::TypedFunc<(WasmPtr, WasmPtr, WasmPtr, WasmPtr), i32>>,
    protect:std::sync::OnceLock<wasmtime::TypedFunc<(WasmPtr, WasmPtr), i32>>,
    exec:std::sync::OnceLock<wasmtime::TypedFunc<(WasmPtr, WasmPtr, WasmPtr, WasmPtr), i32>>,
    memory: std::sync::OnceLock<wasmtime::Memory>,
    instance: std::sync::OnceLock<wasmtime::Instance>,
}


impl PyBoxReactorCore {
    /// 注册一个 Python handler
    /// handle: handler 的 ID
    /// func: Python 可调用对象，接受 bytes 参数，返回 bytes
    fn register_handler(&self, handle: HandleId, func: Py<PyAny>) {
        self.handlers.insert(handle, func);
    }

    /// 取消注册一个 handler
    fn unregister_handler(&self, handle: HandleId) -> bool {
        self.handlers.remove(&handle).is_some()
    }
}

impl PyBoxReactorCore {
    fn new() -> Self {
        Self {
            handlers: dashmap::DashMap::new(),
            ..Default::default()
        }
    }

    // 统一初始化方法
    fn init(
        &self,
        linker: &wasmtime::Linker<WasiP1Ctx>,
        store: &mut wasmtime::Store<WasiP1Ctx>,
        module: &wasmtime::Module,
    ) -> Result<(), String> {
        // 创建 instance
        let instance = linker
            .instantiate(&mut *store, module)
            .map_err(|e| e.to_string())?;

        // 调用 _initialize（如果存在）
        if let Ok(initialize) = instance.get_typed_func::<(), ()>(&mut *store, "_initialize") {
            initialize
                .call(&mut *store, ())
                .map_err(|e| e.to_string())?;
        }

        // 获取并设置 memory
        if let Some(mem) = instance.get_memory(&mut *store, "memory") {
            let _ = self.memory.set(mem);
        }

        // 获取并设置 allocator 和其他函数
        if let (Ok(alloc),
                Ok(free),
                Ok(init_local),
                Ok(init_local_from),
                Ok(del_local),
                Ok(protect),
                Ok(assign),
                Ok(exec),
            ) = (
            instance.get_typed_func::<WasmSize, WasmPtr>(&mut *store, "pybox_alloc_mem"),
            instance.get_typed_func::<WasmPtr, ()>(&mut *store, "pybox_free_mem"),
            instance.get_typed_func::<WasmPtr, i32>(&mut *store, "pybox_init_local"),
            instance.get_typed_func::<(WasmPtr, WasmPtr), i32>(
                &mut *store,
                "pybox_init_local_from",
            ),
            instance.get_typed_func::<WasmPtr, i32>(&mut *store, "pybox_del_local"),
            instance.get_typed_func::<(WasmPtr, WasmPtr), i32>(
                &mut *store,
                "pybox_local_protect",
            ),
            instance.get_typed_func::<(WasmPtr, WasmPtr, WasmPtr, WasmPtr), i32>(
                &mut *store,
                "pybox_assign"
            ),
            instance.get_typed_func::<(WasmPtr, WasmPtr, WasmPtr, WasmPtr), i32>(
                &mut *store,
                "pybox_exec"
            )
        ) {
            let _ = self.alloc_mem.set(alloc);
            let _ = self.free_mem.set(free);
            let _ = self.init_local.set(init_local);
            let _ = self.init_local_from.set(init_local_from);
            let _ = self.del_local.set(del_local);
            let _ = self.protect.set(protect);
            let _ = self.assign.set(assign);
            let _ = self.exec.set(exec);
        }

        // 存储 instance
        self.instance
            .set(instance)
            .map_err(|_| "Failed to set instance".to_string())?;

        Ok(())
    }

    fn get_alloc_mem(&self) -> Option<&wasmtime::TypedFunc<WasmSize, WasmPtr>> {
        self.alloc_mem.get()
    }

    fn get_free_mem(&self) -> Option<&wasmtime::TypedFunc<WasmPtr, ()>> {
        self.free_mem.get()
    }

    pub fn get_memory(&self) -> Option<&wasmtime::Memory> {
        self.memory.get()
    }

    fn get_instance(&self) -> Option<&wasmtime::Instance> {
        self.instance.get()
    }



    // 从 WASM 内存读取字节 (泛型版本，支持 AsContext)
    fn read_memory_bytes(
        &self,
        ctx: impl wasmtime::AsContext<Data = WasiP1Ctx>,
        ptr: WasmPtr,
        len: WasmSize,
    ) -> Result<Vec<u8>, String> {
        let memory = self.get_memory().ok_or("Memory not available")?;
        let mut buffer = vec![0u8; len as usize];
        memory
            .read(ctx, ptr as usize, &mut buffer)
            .map_err(|e| e.to_string())?;
        Ok(buffer)
    }

    // 写入字节到 WASM 内存 (泛型版本，支持 AsContextMut)
    fn write_memory_bytes(
        &self,
        mut ctx: impl wasmtime::AsContextMut<Data = WasiP1Ctx>,
        ptr: WasmPtr,
        data: &[u8],
    ) -> Result<(), String> {
        let memory = self.get_memory().ok_or("Memory not available")?;
        memory
            .write(&mut ctx, ptr as usize, data)
            .map_err(|e| e.to_string())
    }

    // 在 WASM 内存中分配缓冲区 (泛型版本，支持 AsContextMut)
    fn allocate_buffer(
        &self,
        mut ctx: impl wasmtime::AsContextMut<Data = WasiP1Ctx>,
        size: WasmSize,
    ) -> Result<WasmPtr, String> {
        let alloc_func = self
            .get_alloc_mem()
            .expect("pybox_alloc_mem not available - WASM module must export this function");
        alloc_func.call(&mut ctx, size).map_err(|e| e.to_string())
    }

    // 创建一个 pybox_bytes 结构（包含长度和数据）
    fn create_pybox_bytes(
        &self,
        mut ctx: impl wasmtime::AsContextMut<Data = WasiP1Ctx>,
        data: &[u8],
    ) -> Result<WasmPtr, String> {
        // pybox_bytes 的布局: { length: u32, data: [u8; 0] }
        // 总大小 = 4 字节（length）+ 数据长度
        let total_size = 4 + data.len();
        let ptr = self.allocate_buffer(&mut ctx, total_size as WasmSize)?;

        // 写入 length 字段
        self.write_memory_bytes(&mut ctx, ptr, &(data.len() as u32).to_le_bytes())?;

        // 写入 data 字段
        if !data.is_empty() {
            self.write_memory_bytes(&mut ctx, ptr + 4, data)?;
        }

        Ok(ptr)
    }

    // 从 WASM 内存中读取一个 *mut pybox_bytes 指针指向的数据
    fn read_pybox_bytes_ptr(
        &self,
        ctx: impl wasmtime::AsContext<Data = WasiP1Ctx>,
        ptr_ptr: WasmPtr,
    ) -> Result<Option<Vec<u8>>, String> {
        // 读取指针值（4 字节）
        let ptr_bytes = self.read_memory_bytes(&ctx, ptr_ptr, 4)?;
        let ptr = u32::from_le_bytes([ptr_bytes[0], ptr_bytes[1], ptr_bytes[2], ptr_bytes[3]]);

        if ptr == 0 {
            return Ok(None);
        }

        // 读取 pybox_bytes 的 length 字段
        let length_bytes = self.read_memory_bytes(&ctx, ptr, 4)?;
        let length = u32::from_le_bytes([length_bytes[0], length_bytes[1], length_bytes[2], length_bytes[3]]);

        if length == 0 {
            return Ok(Some(Vec::new()));
        }

        // 读取数据
        let data = self.read_memory_bytes(&ctx, ptr + 4, length)?;
        Ok(Some(data))
    }

    // 释放 WASM 内存中的缓冲区 (泛型版本，支持 AsContextMut)
    fn free_buffer(
        &self,
        mut ctx: impl wasmtime::AsContextMut<Data = WasiP1Ctx>,
        ptr: WasmPtr,
    ) -> Result<(), String> {
        let free_func = self
            .get_free_mem()
            .expect("pybox_free_mem not available - WASM module must export this function");
        free_func.call(&mut ctx, ptr).map_err(|e| e.to_string())
    }

    // ==================== 零拷贝优化方法 ====================

    /// 零拷贝读取内存切片（直接返回引用）
    /// 注意：返回的引用生命周期绑定到传入的 context 引用
    fn read_memory_slice<'a>(
        &self,
        ctx: &'a impl wasmtime::AsContext<Data = WasiP1Ctx>,
        ptr: WasmPtr,
        len: WasmSize,
    ) -> Result<&'a [u8], String> {
        let memory = self.get_memory().ok_or("Memory not available")?;
        let memory_data = memory.data(ctx);
        let start = ptr as usize;
        let end = start + len as usize;

        if end > memory_data.len() {
            return Err(format!(
                "Memory access out of bounds: {}..{} > {}",
                start, end, memory_data.len()
            ));
        }

        Ok(&memory_data[start..end])
    }

    /// 零拷贝读取 u32
    fn read_u32(
        &self,
        ctx: &impl wasmtime::AsContext<Data = WasiP1Ctx>,
        ptr: WasmPtr,
    ) -> Result<u32, String> {
        let slice = self.read_memory_slice(ctx, ptr, 4)?;
        Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
    }

    /// 零拷贝读取 pybox_bytes 的数据部分（不包含 length 字段）
    fn read_pybox_bytes_data<'a>(
        &self,
        ctx: &'a impl wasmtime::AsContext<Data = WasiP1Ctx>,
        ptr: WasmPtr,
    ) -> Result<&'a [u8], String> {
        if ptr == 0 {
            return Ok(&[]);
        }

        let length = self.read_u32(ctx, ptr)?;
        if length == 0 {
            return Ok(&[]);
        }

        self.read_memory_slice(ctx, ptr + 4, length)
    }

    /// 零拷贝读取 *mut pybox_bytes 指向的数据
    fn read_pybox_bytes_ptr_data<'a>(
        &self,
        ctx: &'a impl wasmtime::AsContext<Data = WasiP1Ctx>,
        ptr_ptr: WasmPtr,
    ) -> Result<&'a [u8], String> {
        let ptr = self.read_u32(ctx, ptr_ptr)?;
        self.read_pybox_bytes_data(ctx, ptr)
    }

    // ==================== 批量分配优化方法 ====================

    /// 批量分配多个 pybox_bytes 结构，一次性分配连续内存
    ///
    /// 参数：
    /// - data_slices: 要分配的数据切片数组
    ///
    /// 返回：
    /// - Ok((base_ptr, vec![ptr1, ptr2, ...])): 基础指针和各个 pybox_bytes 的指针
    fn allocate_pybox_bytes_batch(
        &self,
        mut ctx: impl wasmtime::AsContextMut<Data = WasiP1Ctx>,
        data_slices: &[&[u8]],
    ) -> Result<(WasmPtr, Vec<WasmPtr>), String> {
        if data_slices.is_empty() {
            return Ok((0, Vec::new()));
        }

        // 1. 计算总大小（每个 pybox_bytes = 4 字节 length + 数据长度）
        let total_size: u32 = data_slices
            .iter()
            .map(|d| 4 + d.len() as u32)
            .sum();

        // 2. 一次性分配整块内存
        let base_ptr = self.allocate_buffer(&mut ctx, total_size)?;

        // 3. 获取内存并批量填充
        let memory = self.get_memory().ok_or("Memory not available")?;
        let memory_data = memory.data_mut(&mut ctx);

        let mut result_ptrs = Vec::with_capacity(data_slices.len());
        let mut offset = 0u32;

        for data in data_slices {
            let ptr = base_ptr + offset;
            let ptr_usize = ptr as usize;
            let len = data.len() as u32;

            // 写入 length 字段
            memory_data[ptr_usize..ptr_usize + 4].copy_from_slice(&len.to_le_bytes());

            // 写入 data 字段
            if len > 0 {
                memory_data[ptr_usize + 4..ptr_usize + 4 + len as usize].copy_from_slice(data);
            }

            result_ptrs.push(ptr);
            offset += 4 + len;
        }

        Ok((base_ptr, result_ptrs))
    }

    // 处理 WASM 的 ioctl 请求
    fn handle_ioctl_request(
        &self,
        mut caller: wasmtime::Caller<'_, WasiP1Ctx>,
        handle: HandleId,
        req_ptr: WasmPtr,
        resp_ptr: WasmPtr,
    ) -> Result<i32, PyErr> {
        pyo3::Python::attach(|py| -> Result<i32, PyErr> {
            // 1. 读取请求包结构
            let memory = match self.get_memory() {
                Some(mem) => mem,
                None => {
                    eprintln!("Memory not available");
                    return Ok(-1);
                }
            };

            let req_packet = match IoctlPacket::read_from_memory(memory, &caller, req_ptr) {
                Ok(packet) => packet,
                Err(e) => {
                    eprintln!("Failed to read request packet: {}", e);
                    return Ok(-1);
                }
            };

            // ========== 优化：零拷贝读取请求数据 ==========
            let req_data = match self.read_memory_slice(&caller, req_packet.buf, req_packet.buf_len) {
                Ok(data) => data,
                Err(e) => {
                    eprintln!("Failed to read request data: {}", e);
                    return Ok(-1);
                }
            };

            // 3. 查找 Python handler
            let handler = match self.handlers.get(&handle) {
                Some(h) => h.clone_ref(py),
                None => return Ok(-1), // Handler 不存在
            };

            // 4. 调用 Python handler（PyBytes::new 内部会拷贝数据，但我们避免了中间 Vec 的分配）
            let req_pybytes = PyBytes::new(py, req_data);
            let resp_result = match handler.call1(py, (req_pybytes,)) {
                Ok(result) => result,
                Err(e) => {
                    // python 异常, 需要传递
                    return Err(e);
                }
            };

            // 5. 提取响应数据（已经是零拷贝：as_bytes 返回引用）
            let resp_bound = resp_result.bind(py);
            let resp_bytes: &pyo3::Bound<'_, PyBytes> = match resp_bound.cast_exact() {
                Ok(bytes) => bytes,
                Err(e) => {
                    eprintln!("Response is not bytes type: {:?}", e);
                    return Ok(-1);
                }
            };
            let resp_data: &[u8] = resp_bytes.as_bytes();

            // 6. 在 WASM 内存中分配响应缓冲区
            let resp_buf_ptr = match self.allocate_buffer(&mut caller, resp_data.len() as u32) {
                Ok(ptr) => ptr,
                Err(e) => {
                    eprintln!("Failed to allocate buffer: {}", e);
                    return Ok(-1);
                }
            };

            // 7. 写入响应数据
            if let Err(e) = self.write_memory_bytes(&mut caller, resp_buf_ptr, resp_data) {
                eprintln!("Failed to write response data: {}", e);
                let _ = self.free_buffer(&mut caller, resp_buf_ptr);
                return Ok(-1);
            }

            // 8. 写入响应包结构
            let resp_packet = IoctlPacket {
                buf: resp_buf_ptr,
                buf_len: resp_data.len() as WasmSize,
            };

            if let Err(e) = resp_packet.write_to_memory(memory, &mut caller, resp_ptr) {
                eprintln!("Failed to write response packet: {}", e);
                let _ = self.free_buffer(&mut caller, resp_buf_ptr);
                return Ok(-1);
            }

            // 9. 返回成功（0 表示成功，非 0 表示失败）
            Ok(0)
        })
    }
}

use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

#[pyclass(subclass)]
pub struct PyBoxReactor {
    pub core: Option<Arc<PyBoxReactorCore>>,
    pub store: Option<std::cell::UnsafeCell<wasmtime::Store<WasiP1Ctx>>>,
    owner_thread_raw: AtomicU64,
}

/// 支持多线程存储
unsafe impl Sync for PyBoxReactor {}

impl PyBoxReactor {

    /// 线程安全访问
    pub fn safe_access<F,R>(&self,f:F) -> pyo3::PyResult<R>
    where F: FnOnce() -> pyo3::PyResult<R> {
        
        let tid: u64 = unsafe { std::mem::transmute(thread::current().id()) };
        // 1. 尝试加锁
        let (is_initial, success) = match self.owner_thread_raw.compare_exchange(
            0, tid, Ordering::SeqCst, Ordering::SeqCst
        ) {
            Ok(_) => (true, true),      // 成功从 0 变 tid：初始调用
            Err(id) if id == tid => (false, true), // 已经是 tid：重入调用
            _ => (false, false),        // 已经是别人的 id：冲突
        };

        if !success {
            return Err(pyo3::exceptions::PyRuntimeError::new_err("PyboxReactor using by another thread!"));
        }

        let result = f();

        // 3. 只有第一层调用负责清零
        if is_initial {
            self.owner_thread_raw.store(0, Ordering::SeqCst);
        }

        result
    }
}

#[pymethods]
impl PyBoxReactor {
    #[new]
    #[pyo3(signature = (*_args, **_kwargs))]
    fn new(_args: &Bound<'_, pyo3::types::PyTuple>, _kwargs: Option<&Bound<'_, pyo3::types::PyDict>>) -> Self {
        Self {
            core: None,
            store: None,
            owner_thread_raw: AtomicU64::new(0),
        }
    }

    /// Initialize the PyBoxReactor instance
    ///
    /// Args:
    ///     wasmfile: Path to the WASM file
    ///     preopen_dirs: Optional dict mapping guest paths to host paths
    #[pyo3(signature = (wasmfile, preopen_dirs=None))]
    fn __init__(
        &mut self,
        wasmfile: &str,
        preopen_dirs: Option<HashMap<String, String>>,
    ) -> pyo3::PyResult<()> {
        // 创建 WASI 上下文构建器
        let mut builder = WasiCtxBuilder::new();

        // 配置 preopen_dirs (虚拟文件系统映射)
        if let Some(dirs) = preopen_dirs {
            for (guest_path, host_path) in dirs {
                builder
                    .preopened_dir(
                        &host_path,
                        &guest_path,
                        wasmtime_wasi::DirPerms::all(),
                        wasmtime_wasi::FilePerms::all(),
                    )
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
            }
        }

        // 构建 WASI Preview 1 上下文
        let wasi_ctx = builder.build_p1();

        // 创建 Store
        let mut store = wasmtime::Store::new(&**DEFAULT_ENGINE, wasi_ctx);

        // 创建 Linker
        let mut linker = wasmtime::Linker::new(&**DEFAULT_ENGINE);

        // 将 WASI Preview 1 添加到 linker
        wasmtime_wasi::preview1::add_to_linker_sync(&mut linker, |s| s)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

        let core = Arc::new(PyBoxReactorCore::new());
        let core_clone = Arc::clone(&core);

        // 添加自定义的符号到 linker
        // (handle: HandleId, req_ptr: WasmPtr, resp_ptr: WasmPtr) -> i32
        // python 异常时继续传递
        linker
            .func_wrap(
                "env",
                "pybox_ioctl_host_req_impl",
                move |caller: wasmtime::Caller<'_, WasiP1Ctx>,
                      handle: HandleId,
                      req_ptr: WasmPtr,
                      resp_ptr: WasmPtr|
                      -> Result<i32, wasmtime::Error> {
                    core_clone.handle_ioctl_request(caller, handle, req_ptr, resp_ptr).map_err(
                        |e| {wasmtime::Error::from(e)}
                    )
                },
            )
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

        // 从缓存加载或编译 WASM 模块
        let cache_key = ModuleCacheKey::new(Arc::clone(&DEFAULT_ENGINE), wasmfile.to_string());

        let module = if let Some(cached) = MODULE_CACHES.get(&cache_key) {
            // 缓存命中，直接使用
            Arc::clone(&cached)
        } else {
            // 缓存未命中，加载并缓存
            let module = Arc::new(
                wasmtime::Module::from_file(&**DEFAULT_ENGINE, wasmfile)
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?,
            );
            MODULE_CACHES.insert(cache_key.clone(), Arc::clone(&module));
            module
        };

        // 使用 core.init 一次性完成所有初始化
        core.init(&linker, &mut store, &module)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;

        // 设置实例的字段
        self.core = Some(core);
        self.store = Some(std::cell::UnsafeCell::new(store));

        Ok(())
    }

    /// Register a Python handler for ioctl requests
    ///
    /// Args:
    ///     handle: Handler ID
    ///     func: Python callable that accepts bytes and returns bytes
    fn register_handler(&self, handle: HandleId, func: Py<PyAny>) -> pyo3::PyResult<()> {
        self.safe_access(|| 
        {
            let core = self.core.as_ref().ok_or_else(|| {
                pyo3::exceptions::PyRuntimeError::new_err("PyBoxReactor not initialized")
            })?;
            core.register_handler(handle, func);
            Ok(())
        })
    }

    /// Unregister a Python handler
    ///
    /// Args:
    ///     handle: Handler ID
    ///
    /// Returns:
    ///     bool: True if handler was found and removed, False otherwise
    fn unregister_handler(&self, handle: HandleId) -> pyo3::PyResult<bool> {
        self.safe_access(|| 
            {
                let core = self.core.as_ref().ok_or_else(|| {
                    pyo3::exceptions::PyRuntimeError::new_err("PyBoxReactor not initialized")
                })?;
                Ok(core.unregister_handler(handle))
            }
        )
    }
    
    
    /// Initialize a new local environment
    ///
    /// Args:
    ///     env_id: Environment ID
    ///
    /// Returns:
    ///     bool: True if successful, False otherwise
    fn init_local(&self, env_id: &str) -> pyo3::PyResult<bool> {
        self.safe_access(||
            {
                let core = self.core.as_ref().ok_or_else(|| {
                    pyo3::exceptions::PyRuntimeError::new_err("PyBoxReactor not initialized")
                })?;

                // 从 UnsafeCell 获取可变指针
                let store_ptr = self.store.as_ref()
                    .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Store not initialized"))?
                    .get();
                let store = unsafe { &mut *store_ptr };

                let pybox_init_local_func = core.init_local.get().ok_or_else(|| {
                    pyo3::exceptions::PyRuntimeError::new_err("Failed to get pybox_init_local")
                })?;

                // ========== 优化：批量分配（虽然只有一个参数，但保持一致性）==========
                let (base_ptr, ptrs) = core
                    .allocate_pybox_bytes_batch(&mut *store, &[env_id.as_bytes()])
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;

                let env_id_ptr = ptrs[0];

                // 调用 WASM 函数
                let result = pybox_init_local_func
                    .call(&mut *store, env_id_ptr)
                    .map_err(|e| {
                        pyo3::exceptions::PyRuntimeError::new_err(format!("pybox_init_local failed: {}", e))
                    })?;

                // 清理
                core.free_buffer(&mut *store, base_ptr)
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;

                Ok(result == 0)
            }
        )
    }

    /// Initialize a new local environment from an existing one
    ///
    /// Args:
    ///     env_id: New environment ID
    ///     from_env_id: Source environment ID to copy from
    ///
    /// Returns:
    ///     bool: True if successful, False otherwise
    fn init_local_from(&self, env_id: &str, from_env_id: &str) -> pyo3::PyResult<bool> {
        self.safe_access(||
            {
                let core = self.core.as_ref().ok_or_else(|| {
                    pyo3::exceptions::PyRuntimeError::new_err("PyBoxReactor not initialized")
                })?;

                // 从 UnsafeCell 获取可变指针
                let store_ptr = self.store.as_ref()
                    .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Store not initialized"))?
                    .get();
                let store = unsafe { &mut *store_ptr };

                let pybox_init_local_from_func = core.init_local_from.get().ok_or_else(|| {
                    pyo3::exceptions::PyRuntimeError::new_err("Failed to get pybox_init_local_from")
                })?;

                // ========== 优化：批量分配两个参数 ==========
                let (base_ptr, ptrs) = core
                    .allocate_pybox_bytes_batch(&mut *store, &[env_id.as_bytes(), from_env_id.as_bytes()])
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;

                let (env_id_ptr, from_env_id_ptr) = (ptrs[0], ptrs[1]);

                // 调用 WASM 函数
                let result = pybox_init_local_from_func
                    .call(&mut *store, (env_id_ptr, from_env_id_ptr))
                    .map_err(|e| {
                        pyo3::exceptions::PyRuntimeError::new_err(format!(
                            "pybox_init_local_from failed: {}",
                            e
                        ))
                    })?;

                // ========== 优化：批量释放（一次调用）==========
                core.free_buffer(&mut *store, base_ptr)
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;

                Ok(result == 0)
            }
        )
    }


        /// Delete a local environment
    ///
    /// Args:
    ///     env_id: Environment ID to delete
    ///
    /// Returns:
    ///     bool: True if successful, False otherwise
    fn del_local(&self, env_id: &str) -> pyo3::PyResult<bool> {
        self.safe_access(|| 
            {
                let core = self.core.as_ref().ok_or_else(|| {
                    pyo3::exceptions::PyRuntimeError::new_err("PyBoxReactor not initialized")
                })?;

                // 从 UnsafeCell 获取可变指针
                let store_ptr = self.store.as_ref()
                    .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Store not initialized"))?
                    .get();
                let store = unsafe { &mut *store_ptr };

                let pybox_del_local_func = core.del_local.get().ok_or_else(|| {
                    pyo3::exceptions::PyRuntimeError::new_err("Failed to get pybox_del_local")
                })?;

                // ========== 优化：批量分配（虽然只有一个参数，但保持一致性）==========
                let (base_ptr, ptrs) = core
                    .allocate_pybox_bytes_batch(&mut *store, &[env_id.as_bytes()])
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;

                let env_id_ptr = ptrs[0];

                // 调用 WASM 函数
                let result = pybox_del_local_func
                    .call(&mut *store, env_id_ptr)
                    .map_err(|e| {
                        pyo3::exceptions::PyRuntimeError::new_err(format!("pybox_del_local failed: {}", e))
                    })?;

                // 清理
                core.free_buffer(&mut *store, base_ptr)
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;

                Ok(result == 0)
            }
        )
    }



    /// Assign a value to a variable in an environment
    ///
    /// Args:
    ///     env_id: Environment ID
    ///     name: Variable name
    ///     value: Value to assign (will be JSON-serialized)
    fn assign(
        &self,
        py: pyo3::Python,
        env_id: &str,
        name: &str,
        value: &Bound<'_, PyAny>,
    ) -> pyo3::PyResult<()> {
        self.safe_access(||
            {
                let core = self.core.as_ref().ok_or_else(|| {
                    pyo3::exceptions::PyRuntimeError::new_err("PyBoxReactor not initialized")
                })?;

                // 从 UnsafeCell 获取可变指针
                let store_ptr = self.store.as_ref()
                    .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Store not initialized"))?
                    .get();
                let store = unsafe { &mut *store_ptr };

                let pybox_assign_func = core.assign.get().ok_or_else(|| {
                    pyo3::exceptions::PyRuntimeError::new_err("Failed to get pybox_assign")
                })?;

                // 将 value 序列化为 JSON
                let json_module = py.import("json")?;
                let json_str: String = json_module.getattr("dumps")?.call1((value,))?.extract()?;

                // ========== 优化：批量分配所有参数 ==========
                let (base_ptr, ptrs) = core
                    .allocate_pybox_bytes_batch(
                        &mut *store,
                        &[
                            env_id.as_bytes(),
                            name.as_bytes(),
                            json_str.as_bytes(),
                            &[0u8; 4], // error_ptr_ptr (初始化为 NULL)
                        ],
                    )
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;

                let (env_id_ptr, name_ptr, json_ptr, error_ptr_ptr) = (ptrs[0], ptrs[1], ptrs[2], ptrs[3]);

                // 调用 WASM 函数
                let result = pybox_assign_func
                    .call(&mut *store, (env_id_ptr, name_ptr, json_ptr, error_ptr_ptr))
                    .map_err(|e| {
                        pyo3::exceptions::PyRuntimeError::new_err(format!("pybox_assign failed: {}", e))
                    })?;

                // ========== 优化：零拷贝读取错误信息 ==========
                let error_msg = {
                    let error_data = core
                        .read_pybox_bytes_ptr_data(&*store, error_ptr_ptr)
                        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;

                    let error_str = String::from_utf8_lossy(error_data).to_string();

                    // 释放 WASM 端分配的错误缓冲区
                    let error_ptr = core
                        .read_u32(&*store, error_ptr_ptr)
                        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;
                    if error_ptr != 0 {
                        core.free_buffer(&mut *store, error_ptr)
                            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;
                    }

                    error_str
                };

                // ========== 优化：批量释放参数（一次调用）==========
                core.free_buffer(&mut *store, base_ptr)
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;

                // 检查结果
                if result != 0 {
                    return Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                        "PyBox assign failed: {}",
                        if !error_msg.is_empty() {
                            error_msg
                        } else {
                            "Unknown error".to_string()
                        }
                    )));
                }

                Ok(())
            }
        )
    }

    

    /// Execute Python code in a sandboxed environment
    ///
    /// Args:
    ///     code: Python code to execute
    ///     env_id: Optional environment ID. If None, uses global environment
    ///
    /// Returns:
    ///     str: Output from the execution (stdout + stderr)
    #[pyo3(signature = (code, env_id=None))]
    fn exec(&self, code: &str, env_id: Option<&str>) -> pyo3::PyResult<String> {
        self.safe_access(|| 
            {
                let core = self.core.as_ref().ok_or_else(|| {
                    pyo3::exceptions::PyRuntimeError::new_err("PyBoxReactor not initialized")
                })?;
                // 从 UnsafeCell 获取可变指针
                let store_ptr = self.store.as_ref()
                    .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Store not initialized"))?
                    .get();

                // 通过 unsafe 创建可变引用
                let store = unsafe { &mut *store_ptr };


                let pybox_exec_func = core.exec.get().ok_or_else(|| {
                    pyo3::exceptions::PyRuntimeError::new_err("Failed to get pybox_exec")
                })?;

                // ========== 优化：批量分配所有参数 ==========
                // 准备输入数据切片
                let mut input_slices = Vec::with_capacity(4);
                let env_id_index = if let Some(env_id) = env_id {
                    input_slices.push(env_id.as_bytes());
                    Some(input_slices.len() - 1)
                } else {
                    None
                };
                input_slices.push(code.as_bytes()); // code
                input_slices.push(&[0u8; 4]); // output_ptr_ptr (初始化为 NULL)
                input_slices.push(&[0u8; 4]); // error_ptr_ptr (初始化为 NULL)

                // 一次性分配所有内存！
                let (base_ptr, ptrs) = core
                    .allocate_pybox_bytes_batch(&mut *store, &input_slices)
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;

                // 解析各个指针
                let (env_id_ptr, code_ptr, output_ptr_ptr, error_ptr_ptr) = if let Some(idx) = env_id_index {
                    (ptrs[idx], ptrs[idx + 1], ptrs[idx + 2], ptrs[idx + 3])
                } else {
                    (0, ptrs[0], ptrs[1], ptrs[2])
                };

                // ========== 调用 WASM 函数 ==========
                let result = pybox_exec_func
                    .call(&mut *store, (env_id_ptr, code_ptr, output_ptr_ptr, error_ptr_ptr))
                    .map_err(|e| match e.downcast::<PyErr>() {
                        Ok(err) => err,
                        Err(err) => pyo3::exceptions::PyRuntimeError::new_err(format!(
                            "Wasmtime runtime error: {}",
                            err
                        )),
                    })?;

                // ========== 优化：零拷贝读取输出 ==========
                let output = {
                    let output_data = core
                        .read_pybox_bytes_ptr_data(&*store, output_ptr_ptr)
                        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;

                    // 从切片引用直接转 String（这里的拷贝是必需的）
                    let output_str = String::from_utf8_lossy(output_data).to_string();

                    // 释放 WASM 端分配的输出缓冲区
                    let output_ptr = core
                        .read_u32(&*store, output_ptr_ptr)
                        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;
                    if output_ptr != 0 {
                        core.free_buffer(&mut *store, output_ptr)
                            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;
                    }

                    output_str
                };

                // ========== 优化：零拷贝读取错误 ==========
                let error = {
                    let error_data = core
                        .read_pybox_bytes_ptr_data(&*store, error_ptr_ptr)
                        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;

                    let error_str = String::from_utf8_lossy(error_data).to_string();

                    // 释放 WASM 端分配的错误缓冲区
                    let error_ptr = core
                        .read_u32(&*store, error_ptr_ptr)
                        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;
                    if error_ptr != 0 {
                        core.free_buffer(&mut *store, error_ptr)
                            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;
                    }

                    error_str
                };

                // ========== 优化：批量释放参数（一次调用）==========
                core.free_buffer(&mut *store, base_ptr)
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;

                // 检查结果
                if result != 0 {
                    return Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                        "PyBox exec failed: {}",
                        if !error.is_empty() {
                            error
                        } else {
                            "Unknown error".to_string()
                        }
                    )));
                }

                Ok(output)
            }
        )

    }

    

    
    /// Protect a variable in an environment (make it read-only from Python code)
    ///
    /// Args:
    ///     env_id: Environment ID
    ///     name: Variable name to protect
    fn protect(&self, env_id: &str, name: &str) -> pyo3::PyResult<()> {
        self.safe_access(||
            {
                let core = self.core.as_ref().ok_or_else(|| {
                    pyo3::exceptions::PyRuntimeError::new_err("PyBoxReactor not initialized")
                })?;

                // 从 UnsafeCell 获取可变指针
                let store_ptr = self.store.as_ref()
                    .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Store not initialized"))?
                    .get();
                let store = unsafe { &mut *store_ptr };

                let pybox_local_protect_func = core.protect.get().ok_or_else(|| {
                    pyo3::exceptions::PyRuntimeError::new_err("Failed to get pybox_protect")
                })?;

                // ========== 优化：批量分配两个参数 ==========
                let (base_ptr, ptrs) = core
                    .allocate_pybox_bytes_batch(&mut *store, &[env_id.as_bytes(), name.as_bytes()])
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;

                let (env_id_ptr, name_ptr) = (ptrs[0], ptrs[1]);

                // 调用 WASM 函数
                let result = pybox_local_protect_func
                    .call(&mut *store, (env_id_ptr, name_ptr))
                    .map_err(|e| {
                        pyo3::exceptions::PyRuntimeError::new_err(format!(
                            "pybox_local_protect failed: {}",
                            e
                        ))
                    })?;

                // ========== 优化：批量释放（一次调用）==========
                core.free_buffer(&mut *store, base_ptr)
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))?;

                // 检查结果
                if result != 0 {
                    return Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                        "Failed to protect variable '{}' in environment '{}'",
                        name, env_id
                    )));
                }

                Ok(())
            }
        )
    }


}
