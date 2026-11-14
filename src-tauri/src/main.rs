// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // ★ P15 · 客户端 hook 会在**每次工具调用**时同步执行本二进制来上报一条事件。
    // 必须在 **GUI 初始化之前**短路掉它：把 Tauri 的窗口、插件、后台循环都拉起来
    // 会吃掉几百毫秒，直接撞上客户端的 hook 超时（外部实现用 3 s）。
    // 返回 `None` = 不是 hook 调用 ⇒ 照常启动桌面应用。
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = buddy_switch_core::modules::agent_hook_rx::try_run_hook_cli(&args) {
        std::process::exit(code);
    }
    buddy_switch_rust_lib::run()
}
