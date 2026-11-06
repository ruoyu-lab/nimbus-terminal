#!/usr/bin/env bash
# 打包成 macOS 应用：target/bundle/Nimbus.app
#
#   scripts/bundle-macos.sh           # 发布版（慢一点，体积小、跑得快）
#   scripts/bundle-macos.sh --debug   # 调试版（编得快）
#
# 做了三件事：编译、把二进制和图标放进 .app 的目录结构、用临时签名（ad-hoc）签一下，
# 这样本机能直接双击打开。要分发给别人需要自己的开发者证书重新签名、公证。
set -euo pipefail

cd "$(dirname "$0")/.."

profile="release"
flags="--release"
if [[ "${1:-}" == "--debug" ]]; then
  profile="debug"
  flags=""
fi

name="Nimbus"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
binary="nimbus-terminal"

cargo build $flags
target_dir="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"

app="target/bundle/${name}.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "${target_dir}/${profile}/${binary}" "$app/Contents/MacOS/${name}"
cp assets/icon.icns "$app/Contents/Resources/icon.icns"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>${name}</string>
  <key>CFBundleDisplayName</key><string>${name}</string>
  <key>CFBundleIdentifier</key><string>com.ruoyu-lab.nimbus</string>
  <key>CFBundleExecutable</key><string>${name}</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
</dict>
</plist>
PLIST

codesign --force --deep --sign - "$app" >/dev/null 2>&1 || echo "（签名失败，不影响本机试用）"
echo "已生成：$app"
