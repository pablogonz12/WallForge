//! Minimal raw EGL FFI (libEGL.so.1) – only what the renderer needs.
#![allow(non_camel_case_types, dead_code)]
use std::ffi::{c_char, c_void, CStr, CString};

pub type EGLDisplay = *mut c_void;
pub type EGLContext = *mut c_void;
pub type EGLSurface = *mut c_void;
pub type EGLConfig = *mut c_void;
pub type EGLImage = *mut c_void;
pub type EGLint = i32;
pub type EGLBoolean = u32;
pub type EGLenum = u32;
pub type EGLAttrib = isize;

pub const EGL_NONE: EGLint = 0x3038;
pub const EGL_PLATFORM_WAYLAND_KHR: EGLenum = 0x31D8;
pub const EGL_OPENGL_ES_API: EGLenum = 0x30A0;
pub const EGL_RENDERABLE_TYPE: EGLint = 0x3040;
pub const EGL_OPENGL_ES3_BIT: EGLint = 0x40;
pub const EGL_SURFACE_TYPE: EGLint = 0x3033;
pub const EGL_WINDOW_BIT: EGLint = 0x4;
pub const EGL_RED_SIZE: EGLint = 0x3024;
pub const EGL_GREEN_SIZE: EGLint = 0x3023;
pub const EGL_BLUE_SIZE: EGLint = 0x3022;
pub const EGL_ALPHA_SIZE: EGLint = 0x3021;
pub const EGL_CONTEXT_CLIENT_VERSION: EGLint = 0x3098;
pub const EGL_WIDTH: EGLint = 0x3057;
pub const EGL_HEIGHT: EGLint = 0x3056;
pub const EGL_EXTENSIONS: EGLint = 0x3055;
pub const EGL_LINUX_DMA_BUF_EXT: EGLenum = 0x3270;
pub const EGL_LINUX_DRM_FOURCC_EXT: EGLint = 0x3271;
pub const EGL_DMA_BUF_PLANE0_FD_EXT: EGLint = 0x3272;
pub const EGL_DMA_BUF_PLANE0_OFFSET_EXT: EGLint = 0x3273;
pub const EGL_DMA_BUF_PLANE0_PITCH_EXT: EGLint = 0x3274;
pub const EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT: EGLint = 0x3443;
pub const EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT: EGLint = 0x3444;

#[link(name = "EGL")]
extern "C" {
    pub fn eglGetProcAddress(name: *const c_char) -> *const c_void;
    pub fn eglGetPlatformDisplay(platform: EGLenum, native: *mut c_void, attribs: *const EGLAttrib) -> EGLDisplay;
    pub fn eglInitialize(d: EGLDisplay, maj: *mut EGLint, min: *mut EGLint) -> EGLBoolean;
    pub fn eglBindAPI(api: EGLenum) -> EGLBoolean;
    pub fn eglChooseConfig(d: EGLDisplay, attr: *const EGLint, cfgs: *mut EGLConfig, n: EGLint, num: *mut EGLint) -> EGLBoolean;
    pub fn eglCreateContext(d: EGLDisplay, c: EGLConfig, share: EGLContext, attr: *const EGLint) -> EGLContext;
    pub fn eglCreatePlatformWindowSurface(d: EGLDisplay, c: EGLConfig, win: *mut c_void, attr: *const EGLAttrib) -> EGLSurface;
    pub fn eglDestroySurface(d: EGLDisplay, s: EGLSurface) -> EGLBoolean;
    pub fn eglMakeCurrent(d: EGLDisplay, dr: EGLSurface, rd: EGLSurface, c: EGLContext) -> EGLBoolean;
    pub fn eglSwapBuffers(d: EGLDisplay, s: EGLSurface) -> EGLBoolean;
    pub fn eglSwapInterval(d: EGLDisplay, i: EGLint) -> EGLBoolean;
    pub fn eglQueryString(d: EGLDisplay, name: EGLint) -> *const c_char;
    pub fn eglGetError() -> EGLint;
    pub fn eglCreateImage(d: EGLDisplay, c: EGLContext, target: EGLenum, buf: *mut c_void, attr: *const EGLAttrib) -> EGLImage;
    pub fn eglDestroyImage(d: EGLDisplay, i: EGLImage) -> EGLBoolean;
}

pub fn has_ext(d: EGLDisplay, ext: &str) -> bool {
    unsafe {
        let p = eglQueryString(d, EGL_EXTENSIONS);
        if p.is_null() { return false; }
        CStr::from_ptr(p).to_string_lossy().split(' ').any(|e| e == ext)
    }
}

pub fn proc_addr(name: &str) -> *const c_void {
    let c = CString::new(name).unwrap();
    unsafe { eglGetProcAddress(c.as_ptr()) }
}
