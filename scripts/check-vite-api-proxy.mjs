#!/usr/bin/env node
// 真实 Vite /api 转发 → 当前 Rust api_guard::judge，防止开发代理破坏同源请求。
// 前置：npm ci、已缓存 Cargo.lock 依赖。运行 node scripts/check-vite-api-proxy.mjs。
// --rewrite-host 仅在测试进程内重现旧配置，预期退出1；不修改 vite.config.ts。
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createServer, loadConfigFromFile } from "vite";

const root = fileURLToPath(new URL("../", import.meta.url));
const temporary = await fs.mkdtemp(path.join(os.tmpdir(), "buddy-switch-api-proxy-"));
const manifest = path.join(temporary, "Cargo.toml");
const probe = path.join(temporary, process.platform === "win32" ? "guard-probe.exe" : "guard-probe");
const rewriteHost = process.argv.includes("--rewrite-host");
let backend;
let frontend;
let vite;

function cargo(args) {
  const result = spawnSync("cargo", args, { cwd: root, encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
  if (result.status !== 0) {
    throw new Error(`Cargo 探针构建失败：${result.error?.message ?? result.stderr}`);
  }
  return result.stdout;
}

async function listen(server) {
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  return `http://127.0.0.1:${server.address().port}`;
}

async function close(server) {
  if (!server?.listening) return;
  server.closeAllConnections();
  await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
}

try {
  // 临时 crate 只依赖真实 guard 所用的两个库；复用本仓锁文件和 Cargo target，
  // 由 Cargo 选择彼此匹配的缓存构件，不猜测 target/debug/deps 中的 hash。
  const metadata = JSON.parse(cargo(["metadata", "--no-deps", "--format-version=1", "--locked", "--offline"]));
  await fs.copyFile(path.join(root, "Cargo.lock"), path.join(temporary, "Cargo.lock"));
  await fs.writeFile(manifest, `[package]
name = "buddy-switch-api-proxy-check"
version = "0.0.0"
edition = "2021"
[[bin]]
name = "buddy-switch-api-proxy-check"
path = "main.rs"
[dependencies]
axum = "0.7"
serde_json = "1"
[profile.dev]
debug = "line-tables-only"
[profile.dev.package."*"]
debug = false
`);
  const guardPath = path.join(root, "crates", "buddy-switch-server", "src", "api_guard.rs");
  await fs.writeFile(path.join(temporary, "main.rs"), `#![allow(dead_code)]
#[path = ${JSON.stringify(guardPath)}]
mod api_guard;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let header = |index: usize| args.get(index).map(String::as_str).filter(|s| !s.is_empty());
    let method = axum::http::Method::from_bytes(args[1].as_bytes()).expect("HTTP method");
    match api_guard::judge(&api_guard::parse_allowed_hosts(None), &method,
        header(2), header(3), header(4), header(5)) {
        Ok(()) => println!("accepted"),
        Err(reason) => {
            println!("rejected:{reason:?}");
            std::process::exit(if reason == api_guard::Deny::NotJson { 4 } else { 3 });
        }
    }
}
`);
  const artifacts = cargo(["build", "--offline", "--manifest-path", manifest,
    "--target-dir", metadata.target_directory, "--message-format=json"])
    .trim().split("\n").map(line => JSON.parse(line));
  const executable = artifacts.find(item => item.target?.name === "buddy-switch-api-proxy-check" && item.executable)?.executable;
  assert.ok(executable, "Cargo 必须返回本轮真实探针的可执行文件");
  await fs.copyFile(executable, probe);
  await fs.chmod(probe, 0o700);

  backend = http.createServer((request, response) => {
    const headers = request.headers;
    const result = spawnSync(probe, [request.method, headers.host ?? "", headers.origin ?? "",
      headers["sec-fetch-site"] ?? "", headers["content-type"] ?? ""], { encoding: "utf8" });
    response.statusCode = result.status === 0 ? 200 : result.status === 3 ? 403 : result.status === 4 ? 415 : 500;
    response.setHeader("content-type", "application/json");
    response.end(JSON.stringify({ host: headers.host, origin: headers.origin,
      guardVerdict: result.stdout?.trim(), guardExit: result.status }));
  });
  const target = await listen(backend);
  const loaded = await loadConfigFromFile({ command: "serve", mode: "development" }, path.join(root, "vite.config.ts"));
  const configured = loaded?.config.server?.proxy?.["/api"];
  assert.ok(configured && typeof configured === "object", "vite.config.ts 必须配置 /api 代理");
  const proxy = { ...configured, target, ...(rewriteHost ? { changeOrigin: true } : {}) };
  // 中间件模式仍执行 Vite 的真实代理链，外层HTTP监听使用系统分配端口，避免占用1420/5173。
  vite = await createServer({ root, configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, hmr: false, watch: null, proxy: { "/api": proxy } } });
  frontend = http.createServer(vite.middlewares);
  const origin = await listen(frontend);
  for (const endpoint of ["/api/auth/totp/validate", "/api/relay/config"]) {
    const response = await fetch(`${origin}${endpoint}`, { method: "POST", signal: AbortSignal.timeout(5_000),
      headers: { origin, "content-type": "application/json", "sec-fetch-site": "same-origin" }, body: "{}" });
    const result = await response.json();
    console.log(JSON.stringify({ endpoint, changeOrigin: proxy.changeOrigin, status: response.status, ...result }));
    assert.equal(response.status, 200, `${endpoint} 必须通过真实 api_guard::judge`);
    assert.equal(result.guardVerdict, "accepted");
    assert.equal(result.host, new URL(origin).host, "代理必须保留浏览器原始 Host");
    assert.equal(result.origin, origin, "代理不应改写浏览器 Origin");
  }
  console.log("[check-vite-api-proxy] 通过：两个管理POST经真实Vite代理和Rust同源校验均返回200");
} catch (error) {
  console.error(`[check-vite-api-proxy] ${error.message}`);
  process.exitCode = 1;
} finally {
  await close(frontend);
  await vite?.close();
  await close(backend);
  // 仅移除本脚本创建的明确文件与空目录，不使用递归删除。
  for (const name of ["Cargo.toml", "Cargo.lock", "main.rs", path.basename(probe)]) {
    await fs.unlink(path.join(temporary, name)).catch(error => { if (error.code !== "ENOENT") throw error; });
  }
  await fs.rmdir(temporary);
}
