#!/bin/bash
set -euo pipefail

APP_NAME="IELTS Author Studio.app"

candidate_paths=(
  "/Applications/$APP_NAME"
  "$HOME/Applications/$APP_NAME"
)

APP_PATH=""
for candidate in "${candidate_paths[@]}"; do
  if [[ -d "$candidate" ]]; then
    APP_PATH="$candidate"
    break
  fi
done

if [[ -z "$APP_PATH" ]]; then
  osascript -e 'display dialog "未在 Applications 中找到 IELTS Author Studio.app。请先将应用从 DMG 拖入 Applications，然后再运行本脚本完成长期放行。" buttons {"好"} default button 1 with icon caution' >/dev/null
  exit 1
fi

echo "Preparing installed app: $APP_PATH"

xattr -cr "$APP_PATH"
xattr -dr com.apple.quarantine "$APP_PATH" 2>/dev/null || true

codesign --force --deep --sign - "$APP_PATH"
spctl --add --label "IELTS Author Studio" "$APP_PATH" 2>/dev/null || true

echo "Opening $APP_NAME..."
open "$APP_PATH"

osascript -e 'display notification "已完成 Applications 内应用的长期放行，并尝试打开应用。" with title "IELTS Author Studio"' >/dev/null 2>&1 || true
