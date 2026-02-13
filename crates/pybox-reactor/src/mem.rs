//! mem.rs for shared memory with host
use libc::{c_void, free, malloc, size_t};

/// 在 pybox 中分配 size_t 大小内存
#[unsafe(no_mangle)]
pub extern "C" fn pybox_alloc_mem(size: size_t) -> *mut c_void {
    unsafe {
        let ptr = malloc(size);
        if ptr.is_null() {
            panic!("allocate memory failed!")
        }
        return ptr;
    }
}

/// 在 pybox 中释放已分配的内存
#[unsafe(no_mangle)]
pub extern "C" fn pybox_free_mem(ptr: *mut c_void) {
    unsafe {
        free(ptr);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_pybox_alloc_and_free() {
        let size = 0x1000;
        let ptr = pybox_alloc_mem(size);
        assert!(!ptr.is_null(), "pybox_alloc_mem returned a null pointer");

        unsafe {
            let slice = std::slice::from_raw_parts_mut(ptr as *mut u8, size);
            slice[0] = 0xAA;
            slice[size - 1] = 0x55;

            assert_eq!(slice[0], 0xAA);
            assert_eq!(slice[size - 1], 0x55);
        }

        pybox_free_mem(ptr);
    }
}
