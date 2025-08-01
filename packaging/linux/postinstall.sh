#!/bin/bash
set -euo pipefail

if command -v systemctl >/dev/null && [ -d /run/systemd/system ]; then
    systemctl daemon-reload
    systemctl enable comenqd.service
    systemctl start comenqd.service
fi
