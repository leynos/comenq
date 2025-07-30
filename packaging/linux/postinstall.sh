#!/bin/bash
set -euo pipefail

systemctl daemon-reload
systemctl enable comenqd.service
systemctl start comenqd.service
