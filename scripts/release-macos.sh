#!/usr/bin/env bash
# 做一个可以发到 GitHub Releases 的包：target/release-assets/ 下面
#   Nimbus-<版本>-macos-<架构>.zip       里面是 Nimbus.app
#   Nimbus-<版本>-macos-<架构>.zip.sha256
#
#   scripts/release-macos.sh
#
# 在本机编译，所以只有本机这种架构（Apple 芯片是 arm64）。用临时签名，没有公证：
# 别人下载后第一次打开要右键 → 打开，或者在终端里执行
#   xattr -dr com.apple.quarantine /Applications/Nimbus.app
set -euo pipefail

cd "$(dirname "$0")/.."

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
arch="$(uname -m)"

scripts/bundle-macos.sh

out="target/release-assets"
zip="Nimbus-${version}-macos-${arch}.zip"
rm -rf "$out"
mkdir -p "$out"
# ditto 保留 .app 里的签名、权限
ditto -c -k --sequesterRsrc --keepParent target/bundle/Nimbus.app "$out/$zip"
(cd "$out" && shasum -a 256 "$zip" > "$zip.sha256")

echo
ls -lh "$out"
cat "$out/$zip.sha256"
