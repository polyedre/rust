// Copyright 2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use ffi::CStr;
use io;
use libc::{c_char, c_ulong};
use mem;
use sys::backtrace::BacktraceContext;
use sys::backtrace::StackWalkVariant;
use sys::c;
use sys_common::backtrace::Frame;

type SymFromInlineContextFn =
    unsafe extern "system" fn(c::HANDLE, u64, c::ULONG, *mut u64, *mut c::SYMBOL_INFO) -> c::BOOL;
type SymGetLineFromInlineContextFn = unsafe extern "system" fn(
    c::HANDLE,
    u64,
    c::ULONG,
    u64,
    *mut c::DWORD,
    *mut c::IMAGEHLP_LINE64,
) -> c::BOOL;

type SymFromAddrFn =
    unsafe extern "system" fn(c::HANDLE, u64, *mut u64, *mut c::SYMBOL_INFO) -> c::BOOL;
type SymGetLineFromAddr64Fn =
    unsafe extern "system" fn(c::HANDLE, u64, *mut u32, *mut c::IMAGEHLP_LINE64) -> c::BOOL;

/// Converts a pointer to symbol to its string value.
pub fn resolve_symname<F>(frame: Frame, callback: F, context: &BacktraceContext) -> io::Result<()>
where
    F: FnOnce(Option<&str>) -> io::Result<()>,
{
    match context.StackWalkVariant {
        StackWalkVariant::StackWalkEx => {
            let SymFromInlineContext =
                sym!(&context.dbghelp, "SymFromInlineContext",SymFromInlineContextFn)?;
            resolve_symname_from_inline_context(SymFromInlineContext, frame, callback, context)
        },
        StackWalkVariant::StackWalk64 => {
            let SymFromAddr = sym!(&context.dbghelp, "SymFromAddr", SymFromAddrFn)?;
            resolve_symname_from_addr(SymFromAddr, frame, callback, context)
        }
    }
}

fn resolve_symname_from_inline_context<F>(
    SymFromInlineContext: SymFromInlineContextFn,
    frame: Frame, callback: F, context: &BacktraceContext) -> io::Result<()>
where
    F: FnOnce(Option<&str>) -> io::Result<()>,
{
    unsafe {
            let mut info: c::SYMBOL_INFO = mem::zeroed();
            info.MaxNameLen = c::MAX_SYM_NAME as c_ulong;
            // the struct size in C.  the value is different to
            // `size_of::<SYMBOL_INFO>() - MAX_SYM_NAME + 1` (== 81)
            // due to struct alignment.
            info.SizeOfStruct = 88;

            let mut displacement = 0u64;
            let ret = SymFromInlineContext(
                context.handle,
                frame.symbol_addr as u64,
                frame.inline_context,
                &mut displacement,
                &mut info,
            );
            let valid_range =
                if ret == c::TRUE && frame.symbol_addr as usize >= info.Address as usize {
                    if info.Size != 0 {
                        (frame.symbol_addr as usize) < info.Address as usize + info.Size as usize
                    } else {
                        true
                    }
                } else {
                    false
                };
            let symname = if valid_range {
                let ptr = info.Name.as_ptr() as *const c_char;
                CStr::from_ptr(ptr).to_str().ok()
            } else {
                None
            };
            callback(symname)
        }
}

fn resolve_symname_from_addr<F>(
    SymFromAddr: SymFromAddrFn,
    frame: Frame, callback: F, context: &BacktraceContext) -> io::Result<()>
where
    F: FnOnce(Option<&str>) -> io::Result<()>,
{
    unsafe {
        let mut info: c::SYMBOL_INFO = mem::zeroed();
        info.MaxNameLen = c::MAX_SYM_NAME as c_ulong;
        // the struct size in C.  the value is different to
        // `size_of::<SYMBOL_INFO>() - MAX_SYM_NAME + 1` (== 81)
        // due to struct alignment.
        info.SizeOfStruct = 88;

        let mut displacement = 0u64;
        let ret = SymFromAddr(
            context.handle,
            frame.symbol_addr as u64,
            &mut displacement,
            &mut info,
        );

        let symname = if ret == c::TRUE {
            let ptr = info.Name.as_ptr() as *const c_char;
            CStr::from_ptr(ptr).to_str().ok()
        } else {
            None
        };
        callback(symname)
    }
}

pub fn foreach_symbol_fileline<F>(
    frame: Frame,
    f: F,
    context: &BacktraceContext,
) -> io::Result<bool>
where
    F: FnMut(&[u8], u32) -> io::Result<()>,
{
    match context.StackWalkVariant {
        StackWalkVariant::StackWalkEx => {
            let SymGetLineFromInlineContext =
                sym!(&context.dbghelp, "SymGetLineFromInlineContext",
                     SymGetLineFromInlineContextFn)?;
            foreach_symbol_fileline_ex(SymGetLineFromInlineContext,
                frame, f, context)
        },
        StackWalkVariant::StackWalk64 => {
            let SymGetLineFromAddr64 =
                sym!(&context.dbghelp, "SymGetLineFromAddr64",
                     SymGetLineFromAddr64Fn)?;
            foreach_symbol_fileline_64(SymGetLineFromAddr64,
                frame, f, context)
        }
    }
}

fn foreach_symbol_fileline_ex<F>(
    SymGetLineFromInlineContext: SymGetLineFromInlineContextFn,
    frame: Frame,
    mut f: F,
    context: &BacktraceContext,
) -> io::Result<bool>
where
    F: FnMut(&[u8], u32) -> io::Result<()>,
{
    unsafe {
        let mut line: c::IMAGEHLP_LINE64 = mem::zeroed();
        line.SizeOfStruct = ::mem::size_of::<c::IMAGEHLP_LINE64>() as u32;

        let mut displacement = 0u32;
        let ret = SymGetLineFromInlineContext(
            context.handle,
            frame.exact_position as u64,
            frame.inline_context,
            0,
            &mut displacement,
            &mut line,
        );
        if ret == c::TRUE {
            let name = CStr::from_ptr(line.Filename).to_bytes();
            f(name, line.LineNumber as u32)?;
        }
        Ok(false)
    }
}

fn foreach_symbol_fileline_64<F>(
    SymGetLineFromAddr64: SymGetLineFromAddr64Fn,
    frame: Frame,
    mut f: F,
    context: &BacktraceContext,
) -> io::Result<bool>
where
    F: FnMut(&[u8], u32) -> io::Result<()>,
{
    unsafe {
        let mut line: c::IMAGEHLP_LINE64 = mem::zeroed();
        line.SizeOfStruct = ::mem::size_of::<c::IMAGEHLP_LINE64>() as u32;

        let mut displacement = 0u32;
        let ret = SymGetLineFromAddr64(
            context.handle,
            frame.exact_position as u64,
            &mut displacement,
            &mut line,
        );
        if ret == c::TRUE {
            let name = CStr::from_ptr(line.Filename).to_bytes();
            f(name, line.LineNumber as u32)?;
        }
        Ok(false)
    }
}
