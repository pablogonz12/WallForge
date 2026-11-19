//! GL ES 3 renderer: samples planar/semi-planar YUV directly in a shader (no RGBA copies).
use crate::egl::*;
use crate::video::{Data, Frame, Matrix};
use std::ffi::{c_void, CString};

#[derive(Clone, Copy, PartialEq)]
pub enum Scaling { Fill, Fit, Stretch }

const VS: &str = "#version 300 es
uniform vec4 rect; out vec2 uv;
void main(){ vec2 p = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
 uv = vec2(p.x, 1.0 - p.y); gl_Position = vec4(mix(rect.xy, rect.zw, p), 0.0, 1.0); }";
const FS: &str = "#version 300 es
precision highp float; in vec2 uv; out vec4 o;
uniform sampler2D ty, tu, tv; uniform int nv12; uniform mat3 m; uniform vec3 off; uniform vec3 sc;
void main(){ float y = texture(ty, uv).r;
 vec2 c = nv12 == 1 ? texture(tu, uv).rg : vec2(texture(tu, uv).r, texture(tv, uv).r);
 o = vec4(clamp(m * ((vec3(y, c) - off) * sc), 0.0, 1.0), 1.0); }";

type TargetFn = unsafe extern "C" fn(u32, *mut c_void);

pub struct Gfx {
    dpy: EGLDisplay,
    prog: u32,
    u_rect: i32, u_nv12: i32, u_m: i32, u_off: i32, u_sc: i32,
    tex: [u32; 3],
    sizes: [(i32, i32, u32); 3],
    target_fn: Option<TargetFn>,
    pub can_dmabuf: bool,
    cur: Option<(u32, u32, bool, Matrix, bool)>, // w,h,nv12,matrix,full
    pub scaling: Scaling,
}

fn shader(kind: u32, src: &str) -> u32 {
    unsafe {
        let s = gl::CreateShader(kind);
        let c = CString::new(src).unwrap();
        gl::ShaderSource(s, 1, &c.as_ptr(), std::ptr::null());
        gl::CompileShader(s);
        let mut ok = 0;
        gl::GetShaderiv(s, gl::COMPILE_STATUS, &mut ok);
        if ok == 0 {
            let mut buf = [0u8; 1024]; let mut n = 0;
            gl::GetShaderInfoLog(s, 1024, &mut n, buf.as_mut_ptr() as *mut _);
            panic!("shader: {}", String::from_utf8_lossy(&buf[..n as usize]));
        }
        s
    }
}

impl Gfx {
    pub fn new(dpy: EGLDisplay, scaling: Scaling) -> Gfx {
        gl::load_with(|n| proc_addr(n) as *const _);
        let can_dmabuf = has_ext(dpy, "EGL_EXT_image_dma_buf_import") && has_ext(dpy, "EGL_EXT_image_dma_buf_import_modifiers");
        let tf = proc_addr("glEGLImageTargetTexture2DOES");
        let target_fn: Option<TargetFn> = if tf.is_null() { None } else { Some(unsafe { std::mem::transmute(tf) }) };
        unsafe {
            let p = gl::CreateProgram();
            gl::AttachShader(p, shader(gl::VERTEX_SHADER, VS));
            gl::AttachShader(p, shader(gl::FRAGMENT_SHADER, FS));
            gl::LinkProgram(p);
            gl::UseProgram(p);
            let loc = |n: &str| { let c = CString::new(n).unwrap(); gl::GetUniformLocation(p, c.as_ptr()) };
            gl::Uniform1i(loc("ty"), 0); gl::Uniform1i(loc("tu"), 1); gl::Uniform1i(loc("tv"), 2);
            let mut tex = [0u32; 3];
            gl::GenTextures(3, tex.as_mut_ptr());
            for (i, t) in tex.iter().enumerate() {
                gl::ActiveTexture(gl::TEXTURE0 + i as u32);
                gl::BindTexture(gl::TEXTURE_2D, *t);
                gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
                gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
                gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
                gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
            }
            gl::Disable(gl::BLEND); gl::Disable(gl::DEPTH_TEST); gl::Disable(gl::DITHER);
            gl::PixelStorei(gl::UNPACK_ALIGNMENT, 1);
            Gfx { dpy, prog: p, u_rect: loc("rect"), u_nv12: loc("nv12"), u_m: loc("m"), u_off: loc("off"), u_sc: loc("sc"),
                  tex, sizes: [(0, 0, 0); 3], target_fn, can_dmabuf: can_dmabuf && target_fn.is_some(), cur: None, scaling }
        }
    }

    /// Upload (or import) a frame into the plane textures. Err => caller should fall back to software.
    pub fn upload(&mut self, f: &Frame) -> Result<(), String> {
        let nv12;
        match &f.data {
            Data::Drm(d) => {
                if !self.can_dmabuf { return Err("no dma-buf import".into()); }
                let desc = d.desc();
                if desc.nb_layers != 2 { return Err(format!("unexpected layer count {}", desc.nb_layers)); }
                for i in 0..2usize {
                    let l = &desc.layers[i];
                    if l.nb_planes != 1 { return Err("multi-plane layer".into()); }
                    let pl = &l.planes[0];
                    let obj = &desc.objects[pl.object_index as usize];
                    let (w, h) = if i == 0 { (f.width, f.height) } else { ((f.width + 1) / 2, (f.height + 1) / 2) };
                    let at: [EGLAttrib; 17] = [
                        EGL_WIDTH as isize, w as isize, EGL_HEIGHT as isize, h as isize,
                        EGL_LINUX_DRM_FOURCC_EXT as isize, l.format as isize,
                        EGL_DMA_BUF_PLANE0_FD_EXT as isize, obj.fd as isize,
                        EGL_DMA_BUF_PLANE0_OFFSET_EXT as isize, pl.offset,
                        EGL_DMA_BUF_PLANE0_PITCH_EXT as isize, pl.pitch,
                        EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT as isize, (obj.modifier & 0xffff_ffff) as isize,
                        EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT as isize, (obj.modifier >> 32) as isize,
                        EGL_NONE as isize,
                    ];
                    unsafe {
                        let img = eglCreateImage(self.dpy, std::ptr::null_mut(), EGL_LINUX_DMA_BUF_EXT, std::ptr::null_mut(), at.as_ptr());
                        if img.is_null() { return Err(format!("eglCreateImage plane {i} failed: 0x{:x}", eglGetError())); }
                        gl::ActiveTexture(gl::TEXTURE0 + i as u32);
                        gl::BindTexture(gl::TEXTURE_2D, self.tex[i]);
                        (self.target_fn.unwrap())(gl::TEXTURE_2D, img);
                        eglDestroyImage(self.dpy, img);
                    }
                    self.sizes[i] = (0, 0, 0); // storage now belongs to the EGLImage
                }
                nv12 = true;
            }
            Data::Sw(v) => {
                use ffmpeg_next::format::Pixel;
                nv12 = v.format() == Pixel::NV12;
                let n = if nv12 { 2 } else { 3 };
                for i in 0..n {
                    let (w, h) = (v.plane_width(i) as i32, v.plane_height(i) as i32);
                    let (ifmt, fmt) = if nv12 && i == 1 { (gl::RG8, gl::RG) } else { (gl::R8, gl::RED) };
                    let bpp = if nv12 && i == 1 { 2 } else { 1 };
                    unsafe {
                        gl::ActiveTexture(gl::TEXTURE0 + i as u32);
                        gl::BindTexture(gl::TEXTURE_2D, self.tex[i]);
                        gl::PixelStorei(gl::UNPACK_ROW_LENGTH, (v.stride(i) / bpp) as i32);
                        if self.sizes[i] != (w, h, ifmt) {
                            gl::TexImage2D(gl::TEXTURE_2D, 0, ifmt as i32, w, h, 0, fmt, gl::UNSIGNED_BYTE, v.data(i).as_ptr() as *const _);
                            self.sizes[i] = (w, h, ifmt);
                        } else {
                            gl::TexSubImage2D(gl::TEXTURE_2D, 0, 0, 0, w, h, fmt, gl::UNSIGNED_BYTE, v.data(i).as_ptr() as *const _);
                        }
                        gl::PixelStorei(gl::UNPACK_ROW_LENGTH, 0);
                    }
                }
            }
        }
        self.cur = Some((f.width, f.height, nv12, f.matrix, f.full_range));
        Ok(())
    }

    /// Draw the last uploaded frame to the currently bound surface.
    pub fn draw(&self, out_w: i32, out_h: i32, native: bool) {
        let Some((w, h, nv12, matrix, full)) = self.cur else { return };
        let (kr, kb) = match matrix { Matrix::Bt709 => (0.2126f32, 0.0722f32), Matrix::Bt601 => (0.299, 0.114), Matrix::Bt2020 => (0.2627, 0.0593) };
        let kg = 1.0 - kr - kb;
        let m = [1.0, 1.0, 1.0, 0.0, -2.0 * kb * (1.0 - kb) / kg, 2.0 * (1.0 - kb), 2.0 * (1.0 - kr), -2.0 * kr * (1.0 - kr) / kg, 0.0f32];
        let (off, sc) = if full { ([0.0, 0.5, 0.5f32], [1.0, 1.0, 1.0f32]) }
            else { ([16.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0], [255.0 / 219.0, 255.0 / 224.0, 255.0 / 224.0]) };
        let va = w as f32 / h as f32;
        let oa = out_w as f32 / out_h as f32;
        let (sx, sy) = if native { (1.0, 1.0) } else { match self.scaling {
            Scaling::Stretch => (1.0, 1.0),
            Scaling::Fill => if va > oa { (va / oa, 1.0) } else { (1.0, oa / va) },
            Scaling::Fit => if va > oa { (1.0, oa / va) } else { (va / oa, 1.0) },
        } };
        unsafe {
            gl::Viewport(0, 0, out_w, out_h);
            if self.scaling == Scaling::Fit { gl::ClearColor(0.0, 0.0, 0.0, 1.0); gl::Clear(gl::COLOR_BUFFER_BIT); }
            gl::UseProgram(self.prog);
            gl::Uniform4f(self.u_rect, -sx, -sy, sx, sy);
            gl::Uniform1i(self.u_nv12, nv12 as i32);
            gl::UniformMatrix3fv(self.u_m, 1, gl::FALSE, m.as_ptr());
            gl::Uniform3fv(self.u_off, 1, off.as_ptr());
            gl::Uniform3fv(self.u_sc, 1, sc.as_ptr());
            for i in 0..3 { gl::ActiveTexture(gl::TEXTURE0 + i); gl::BindTexture(gl::TEXTURE_2D, self.tex[i as usize]); }
            gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);
        }
    }
}
