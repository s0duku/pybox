//! ioctl.rs implement pybox communicate with host

use libc::{c_void, size_t, ssize_t};

#[repr(C, packed)]
pub struct pybox_bytes {
    pub length: size_t,
    pub data: [u8; 0]
}

impl pybox_bytes {
    
    pub fn new_bytes(bytes: &[u8]) -> *mut Self {
        use crate::mem::pybox_alloc_mem;
        let len = bytes.len();
        // 1. 计算总大小：结构体本身大小 + 数据的长度
        let size = std::mem::size_of::<Self>() + len;  
        // 2. 分配内存
        unsafe {
            let ptr = pybox_alloc_mem(size) as *mut Self;
            if !ptr.is_null() {
                // 3. 初始化 length 字段
                (*ptr).length = len;
                // 4. 将数据拷贝到 data 字段之后的内存区域
                let data_ptr = std::ptr::addr_of_mut!((*ptr).data) as *mut u8;
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), data_ptr, len);
        
            }
            ptr
        }
    }

    pub fn string(&self) -> Result<&str,()> {
        unsafe {
            let slice = std::slice::from_raw_parts(self.data.as_ptr(), self.length);
            match std::str::from_utf8(slice) {
                Ok(s) => Ok(s),
                _ => Err(())
            }
        }
    }

}

#[repr(C, packed)]
pub struct pybox_ioctl_packet {
    pub buf: *mut c_void,
    pub buf_len: size_t,
}

#[cfg(target_arch = "wasm32")]
unsafe extern "C" {
    pub fn pybox_ioctl_host_req_impl(
        handle: size_t,
        req: *mut pybox_ioctl_packet,
        resp: *mut pybox_ioctl_packet,
    ) -> ssize_t;
}

#[cfg(not(target_arch = "wasm32"))]
pub fn pybox_ioctl_host_req_impl(
    handle: size_t,
    req: *mut pybox_ioctl_packet,
    resp: *mut pybox_ioctl_packet,
) -> ssize_t {
    // mock
    let _ = handle;
    let _ = req;
    let _ = resp;
    0
}
