#[allow(
    non_snake_case,
    non_upper_case_globals,
    non_camel_case_types,
    dead_code,
    clippy::all
)]
pub mod loader;

pub mod Microsoft {
    pub mod Web {
        pub mod WebView2 {
            pub mod Win32 {
                mod windows_core {
                    macro_rules! link_webview2 {
                        ($library:literal $abi:literal fn $function:ident ($($arg:ident : $arg_ty:ty),* $(,)?) -> $ret_ty:ty) => {
                            #[allow(non_snake_case)]
                            pub(crate) unsafe fn $function($($arg: $arg_ty),*) -> $ret_ty {
                                type FnType = unsafe extern $abi fn($($arg_ty),*) -> $ret_ty;
                                let proc_name = concat!(stringify!($function), "\0");
                                if let Some(p) = $crate::loader::get_proc(proc_name.as_bytes()) {
                                    let func: FnType = core::mem::transmute(p);
                                    func($($arg),*)
                                } else {
                                    windows_core::HRESULT(-2147467259)
                                }
                            }
                        };
                    }

                    pub(crate) use ::windows_core::*;
                    pub(crate) use link_webview2 as link;
                }

                include!("bindings.rs");
            }
        }
    }
}

pub mod declared_interfaces;
