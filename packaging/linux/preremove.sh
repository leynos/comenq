#!/bin/bash
set -euo pipefail

if systemctl is-active --quiet comenqd.service; then
    systemctl stop comenqd.service
fi

if systemctl is-enabled --quiet comenqd.service; then
    systemctl disable comenqd.service
fi
