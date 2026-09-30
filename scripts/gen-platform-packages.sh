#!/bin/bash
# 生成平台包 package.json（esbuild 模式：每个平台一个 npm 包，从 npm registry 下载二进制）
# 用法：sh scripts/gen-platform-packages.sh <版本号，如 2026.9.211636>
#
# **目录名不带 scope、包名带 scope**：目录是 `buddy-switch-<tag>`（`.github/workflows/build.yml`
# 里的 `PKG_DIR` 直接拼这个路径），而 npm 包名是 `@kingkadienm/buddy-switch-<tag>`
# ——主包 `@kingkadienm/buddy-switch` 的 `optionalDependencies` 与 `npm/scripts/install.js`
# 都按**包名**查找，两者不要混。
set -e
V=$1
[ -z "$V" ] && echo "用法: sh scripts/gen-platform-packages.sh <版本号>" && exit 1

cd "$(dirname "$0")/../npm/platform" || exit 1

gen() {
  local tag="$1" os="$2" cpu="$3" binfile="$4"
  local dir="buddy-switch-$tag"
  mkdir -p "$dir/bin"
  cat > "$dir/package.json" << JSON
{
  "name": "@kingkadienm/buddy-switch-$tag",
  "version": "$V",
  "description": "Buddy Switch platform binary ($tag)",
  "os": ["$os"],
  "cpu": ["$cpu"],
  "files": ["bin"],
  "license": "PolyForm-Noncommercial-1.0.0"
}
JSON
  echo "生成 $dir (包名 @kingkadienm/buddy-switch-$tag, bin=$binfile)"
}

gen darwin-arm64 darwin arm64 buddy-switch-darwin-arm64
gen darwin-x64 darwin x64 buddy-switch-darwin-x64
gen win32-x64 win32 x64 buddy-switch-win32-x64.exe
gen linux-x64 linux x64 buddy-switch-linux-x64
gen linux-arm64 linux arm64 buddy-switch-linux-arm64

echo "平台包生成完成（版本 $V），把对应二进制复制到各包 bin/ 后 npm publish。"
