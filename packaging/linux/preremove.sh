#!/bin/bash
set -euo pipefail

if command -v systemctl >/dev/null && [ -d /run/systemd/system ]; then
    if systemctl is-active --quiet comenqd.service; then
        systemctl stop comenqd.service
    fi
    if systemctl is-enabled --quiet comenqd.service; then
        systemctl disable comenqd.service
    fi
fi
