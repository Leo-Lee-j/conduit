#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let result = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("缺少更新交接文件"))
        .and_then(|path| conduit::update::install::run(std::path::Path::new(&path)));
    if let Err(error) = result {
        let text: Vec<u16> = format!("Conduit 更新未完成：{error:#}")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let title: Vec<u16> = "Conduit 更新".encode_utf16().chain(Some(0)).collect();
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                windows_sys::Win32::UI::WindowsAndMessaging::MB_OK
                    | windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
        }
        std::process::exit(1);
    }
}
