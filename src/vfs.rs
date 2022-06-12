use esp_idf_sys::c_types::c_void;
use esp_idf_sys::*;
use std::ffi::CStr;
use std::{ptr, slice};

pub type FsResult<T> = Result<T, i32>;

pub struct Dirent {
    ino: i32,
    file_type: u8,
    name: String,
}

pub trait FileSystem {
    type Dir: Directory;

    fn write(&mut self, fd: i32, data: &[u8]) -> FsResult<usize>;
    fn lseek(&mut self, fd: i32, size: isize, mode: i32) -> FsResult<usize>;
    fn read(&mut self, fd: i32, dst: &mut [u8]) -> FsResult<usize>;
    fn pread(&mut self, fd: i32, dst: &mut [u8], offset: isize) -> FsResult<usize>;
    fn pwrite(&mut self, fd: i32, data: &[u8], offset: isize) -> FsResult<usize>;
    fn open(&mut self, path: &CStr, flags: i32, mode: i32) -> FsResult<i32>;
    fn close(&mut self, fd: i32) -> FsResult<()>;
    // fn fstat(&mut self, fd: i32, ) -> FsResult<()>;
    // fn stat(&mut self, path: &CStr, ) -> FsResult<()>;
    fn link(&mut self, n1: &CStr, n2: &CStr) -> FsResult<()>;
    fn unlink(&mut self, path: &CStr) -> FsResult<()>;
    fn rename(&mut self, src: &CStr, dst: &CStr) -> FsResult<()>;
    fn opendir(&mut self, name: &CStr) -> FsResult<Self::Dir>;
    fn mkdir(&mut self, name: &CStr, mode: u32) -> FsResult<()>;
    fn rmdir(&mut self, name: &CStr) -> FsResult<()>;
    // fn fcntl(&mut self, fd: i32, cmd: i32, arg: i32) -> FsResult<_>;
    // fn ioctl(&mut self, fd: i32, cmd: i32, arg: _) -> FsResult<_>;
    fn fsync(&mut self, fd: i32) -> FsResult<()>;
    fn access(&mut self, path: &CStr, amode: i32) -> FsResult<()>;
    fn truncate(&mut self, path: &CStr, length: usize) -> FsResult<()>;
}

pub trait Directory {
    fn readdir(&mut self) -> FsResult<Dirent>;
    fn telldir(&mut self) -> FsResult<isize>;
    fn seekdir(&mut self, offset: isize) -> FsResult<()>;
    fn closedir(self) -> FsResult<()>;
}

#[repr(C)]
struct DirObj<T: Directory> {
    dir: DIR,
    directory: T,
}

fn do_stuff<F: FileSystem>() {
    fn cast_to_fs(_p: *mut c_void) -> &mut F {
        todo!()
    }

    let _vfs_def = esp_vfs_t {
        flags: ESP_VFS_FLAG_CONTEXT_PTR as _,
        __bindgen_anon_1: esp_vfs_t__bindgen_ty_1 {
            write_p: Some(|p, fd, data, size| {
                let fs = cast_to_fs(p);
                let data = unsafe { slice::from_raw_parts(data as *const u8, size as usize) };
                let result = fs.write(fd, data);
                match result {
                    Ok(value) => value as _,
                    Err(_errno) => -1,
                }
            }),
        },
        __bindgen_anon_2: esp_vfs_t__bindgen_ty_2 {
            lseek_p: Some(|p, fd, size, mode| {
                let fs = cast_to_fs(p);
                let result = fs.lseek(fd, size as isize, mode);
                match result {
                    Ok(value) => value as _,
                    Err(_errno) => -1,
                }
            }),
        },
        __bindgen_anon_3: esp_vfs_t__bindgen_ty_3 {
            read_p: Some(|p, fd, dst, size| {
                let fs = cast_to_fs(p);
                let data = unsafe { slice::from_raw_parts_mut(data as *mut u8, size as usize) };
                let result = fs.read(fd, data);
                match result {
                    Ok(value) => value as _,
                    Err(_errno) => -1,
                }
            }),
        },
        __bindgen_anon_4: esp_vfs_t__bindgen_ty_4 {
            pread_p: Some(|p, fd, dst, size, offset| {
                let fs = cast_to_fs(p);
                let data = unsafe { slice::from_raw_parts_mut(data as *mut u8, size as usize) };
                let result = fs.pread(fd, data, offset as _);
                match result {
                    Ok(value) => value as _,
                    Err(_errno) => -1,
                }
            }),
        },
        __bindgen_anon_5: esp_vfs_t__bindgen_ty_5 {
            pwrite_p: Some(|p, fd, src, size, offset| {
                let fs = cast_to_fs(p);
                let data = unsafe { slice::from_raw_parts(src as *const u8, size as usize) };
                let result = fs.pwrite(fd, data, offset as _);
                match result {
                    Ok(value) => value as _,
                    Err(_errno) => -1,
                }
            }),
        },
        __bindgen_anon_6: esp_vfs_t__bindgen_ty_6 {
            open_p: Some(|p, path, flags, mode| {
                let fs = cast_to_fs(p);
                let path = unsafe { CStr::from_ptr(path) };
                let result = fs.open(path, flags, mode);
                match result {
                    Ok(value) => value as _,
                    Err(_errno) => -1,
                }
            }),
        },
        __bindgen_anon_7: esp_vfs_t__bindgen_ty_7 {
            close_p: Some(|p, fd| {
                let fs = cast_to_fs(p);
                let result = fs.close(fd);
                match result {
                    Ok(_) => 0,
                    Err(_errno) => -1,
                }
            }),
        },
        __bindgen_anon_8: esp_vfs_t__bindgen_ty_8 { fstat_p: None },
        __bindgen_anon_9: esp_vfs_t__bindgen_ty_9 { stat_p: None },
        __bindgen_anon_10: esp_vfs_t__bindgen_ty_10 {
            link_p: Some(|p, n1, n2| {
                let fs = cast_to_fs(ctx);
                let n1 = unsafe { CStr::from_ptr(n1) };
                let n2 = unsafe { CStr::from_ptr(n2) };
                let result = fs.link(n1, n2);
                match result {
                    Ok(_) => 0,
                    Err(_errno) => -1,
                }
            }),
        },
        __bindgen_anon_11: esp_vfs_t__bindgen_ty_11 {
            unlink_p: Some(|p, path| {
                let fs = cast_to_fs(ctx);
                let path = unsafe { CStr::from_ptr(path) };
                let result = fs.unlink(path);
                match result {
                    Ok(_) => 0,
                    Err(_errno) => -1,
                }
            }),
        },
        __bindgen_anon_12: esp_vfs_t__bindgen_ty_12 {
            rename_p: Some(|ctx, src, dst| {
                let fs = cast_to_fs(ctx);
                let src = unsafe { CStr::from_ptr(src) };
                let dst = unsafe { CStr::from_ptr(dst) };
                let result = fs.rename(src, dst);
                match result {
                    Ok(_) => 0,
                    Err(_errno) => -1,
                }
            }),
        },
        __bindgen_anon_13: esp_vfs_t__bindgen_ty_13 {
            opendir_p: Some(|ctx, name| {
                let fs = cast_to_fs(ctx);
                let name = unsafe { CStr::from_ptr(name) };
                let result = fs.opendir(name);
                match result {
                    Ok(directory) => {
                        let dir_obj = Box::new(DirObj {
                            dir: Default::default(),
                            directory,
                        });
                        let raw = Box::into_raw(dir_obj);
                        raw as *mut DIR
                    }
                    Err(_errno) => ptr::null_mut(),
                }
            }),
        },
        __bindgen_anon_14: esp_vfs_t__bindgen_ty_14 {
            readdir_p: Some(|_, pdir| {
                let raw_dir = pdir as *mut DirObj<F::Dir>;
                let dir_obj = unsafe { &mut *raw_dir };
                let result = dir_obj.directory.readdir();
                match result {
                    Ok(_) => ptr::null_mut(),
                    Err(_errno) => ptr::null_mut(),
                }
            }),
        },
        __bindgen_anon_15: esp_vfs_t__bindgen_ty_15 {
            readdir_r_p: None, // Deprecated
        },
        __bindgen_anon_16: esp_vfs_t__bindgen_ty_16 {
            telldir_p: Some(|_, pdir| {
                let raw_dir = pdir as *mut DirObj<F::Dir>;
                let dir_obj = unsafe { &mut *raw_dir };
                let result = dir_obj.directory.telldir();
                match result {
                    Ok(value) => value as _,
                    Err(_errno) => -1,
                }
            }),
        },
        __bindgen_anon_17: esp_vfs_t__bindgen_ty_17 {
            seekdir_p: Some(|_, pdir, offset| {
                let raw_dir = pdir as *mut DirObj<F::Dir>;
                let dir_obj = unsafe { &mut *raw_dir };
                let result = dir_obj.directory.seekdir(offset as _);
                if let Err(_errno) = result {}
            }),
        },
        __bindgen_anon_18: esp_vfs_t__bindgen_ty_18 {
            closedir_p: Some(|_, pdir| {
                let raw_dir = pdir as *mut DirObj<F::Dir>;
                let dir_obj = unsafe { Box::from_raw(raw_dir) };
                let result = dir_obj.directory.closedir();
                if let Err(_errno) = result {
                    -1
                } else {
                    0
                }
            }),
        },
        __bindgen_anon_19: esp_vfs_t__bindgen_ty_19 {
            mkdir_p: Some(|ctx, name, mode| {
                let fs = cast_to_fs(ctx);
                let name = unsafe { CStr::from_ptr(name) };
                let result = fs.mkdir(name, mode as _);
                if let Err(_errno) = result {
                    -1
                } else {
                    0
                }
            }),
        },
        __bindgen_anon_20: esp_vfs_t__bindgen_ty_20 {
            rmdir_p: Some(|ctx, name| {
                let fs = cast_to_fs(ctx);
                let name = unsafe { CStr::from_ptr(name) };
                let result = fs.rmdir(name);
                if let Err(_errno) = result {
                    -1
                } else {
                    0
                }
            }),
        },
        __bindgen_anon_21: esp_vfs_t__bindgen_ty_21 { fcntl_p: None },
        __bindgen_anon_22: esp_vfs_t__bindgen_ty_22 { ioctl_p: None },
        __bindgen_anon_23: esp_vfs_t__bindgen_ty_23 {
            fsync_p: Some(|ctx, fd| {
                let fs = cast_to_fs(ctx);
                let result = fs.fsync(fd);
                if let Err(_errno) = result {
                    -1
                } else {
                    0
                }
            }),
        },
        __bindgen_anon_24: esp_vfs_t__bindgen_ty_24 {
            access_p: Some(|ctx, path, amode| {
                let fs = cast_to_fs(ctx);
                let path = unsafe { CStr::from_ptr(path) };
                let result = fs.access(path, amode);
                if let Err(_errno) = result {
                    -1
                } else {
                    0
                }
            }),
        },
        __bindgen_anon_25: esp_vfs_t__bindgen_ty_25 {
            truncate_p: Some(|ctx, path, length| {
                let fs = cast_to_fs(ctx);
                let path = unsafe { CStr::from_ptr(path) };
                let result = fs.truncate(path, length as _);
                if let Err(_errno) = result {
                    -1
                } else {
                    0
                }
            }),
        },
        __bindgen_anon_26: esp_vfs_t__bindgen_ty_26 { utime_p: None },
        __bindgen_anon_27: esp_vfs_t__bindgen_ty_27 { tcsetattr_p: None },
        __bindgen_anon_28: esp_vfs_t__bindgen_ty_28 { tcgetattr_p: None },
        __bindgen_anon_29: esp_vfs_t__bindgen_ty_29 { tcdrain_p: None },
        __bindgen_anon_30: esp_vfs_t__bindgen_ty_30 { tcflush_p: None },
        __bindgen_anon_31: esp_vfs_t__bindgen_ty_31 { tcflow_p: None },
        __bindgen_anon_32: esp_vfs_t__bindgen_ty_32 { tcgetsid_p: None },
        __bindgen_anon_33: esp_vfs_t__bindgen_ty_33 {
            tcsendbreak_p: None,
        },
        start_select: None,
        socket_select: None,
        stop_socket_select: None,
        stop_socket_select_isr: None,
        get_socket_select_semaphore: None,
        end_select: None,
    };
}
