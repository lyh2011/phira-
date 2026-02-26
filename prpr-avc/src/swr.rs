use crate::{ffi, handle, AudioStreamFormat, Error, OwnedPtr, Result};
use std::ptr::null_mut;

pub struct SwrContext(OwnedPtr<ffi::SwrContext>);
impl SwrContext {
    pub fn new(in_format: &AudioStreamFormat, out_format: &AudioStreamFormat) -> Result<Self> {
        unsafe {
            OwnedPtr::new(ffi::swr_alloc_set_opts(
                null_mut(),
                out_format.channel_layout as _,
                out_format.sample_fmt,
                out_format.sample_rate,
                in_format.channel_layout as _,
                in_format.sample_fmt,
                in_format.sample_rate,
                0,
                null_mut(),
            ))
            .map(Self)
            .ok_or(Error::AllocationFailed)
        }
    }

    pub fn new_high_quality(in_format: &AudioStreamFormat, out_format: &AudioStreamFormat) -> Result<Self> {
        unsafe {
            let ctx = ffi::swr_alloc_set_opts(
                null_mut(),
                out_format.channel_layout as _,
                out_format.sample_fmt,
                out_format.sample_rate,
                in_format.channel_layout as _,
                in_format.sample_fmt,
                in_format.sample_rate,
                0,
                null_mut(),
            );
            
            if ctx.is_null() {
                return Err(Error::AllocationFailed);
            }
            
            // 设置最高质量的重采样参数
            // SWR_FILTER_TYPE_KAISER = 2 (最高质量的滤波器)
            ffi::av_opt_set_int(ctx as *mut _, b"filter_type\0".as_ptr() as *const _, 2, 0);
            // 设置 Kaiser 窗口 beta 值为 16 (更高的值 = 更好的质量)
            ffi::av_opt_set_int(ctx as *mut _, b"kaiser_beta\0".as_ptr() as *const _, 16, 0);
            // 设置重采样引擎为 soxr (最高质量)
            ffi::av_opt_set_int(ctx as *mut _, b"resampler\0".as_ptr() as *const _, 1, 0);
            
            OwnedPtr::new(ctx).map(Self).ok_or(Error::AllocationFailed)
        }
    }

    pub fn init(&mut self) -> Result<()> {
        unsafe { handle(ffi::swr_init(self.0.as_mut())) }
    }

    pub fn get_delay(&self, base: i32) -> i64 {
        unsafe { ffi::swr_get_delay(self.0.as_ref(), base) }
    }

    pub fn convert(&mut self, in_frame: *const u8, in_count: i32, mut out_frame: *mut u8, out_count: i32) -> Result<usize> {
        unsafe {
            let old_out_frame = out_frame;
            let res = ffi::swr_convert(self.0.as_mut(), &mut out_frame as *mut *mut _, out_count, &in_frame as *const *const _, in_count);
            assert_eq!(old_out_frame, out_frame, "reallocation ocurred");

            if res < 0 {
                Err(Error::from_error_code(res))
            } else {
                Ok(res as usize)
            }
        }
    }
}

unsafe impl Send for SwrContext {}
unsafe impl Sync for SwrContext {}

impl Drop for SwrContext {
    fn drop(&mut self) {
        unsafe { ffi::swr_free(self.0.as_self_mut()) }
    }
}
