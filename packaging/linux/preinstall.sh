#!/bin/bash
set -euo pipefail

if ! getent group comenq >/dev/null; then
    groupadd --system comenq || {
        echo "Failed to create group" >&2
        exit 1
    }
fi

if ! getent passwd comenq >/dev/null; then
    useradd --system --gid comenq --home-dir /var/lib/comenq \
        --create-home --shell /sbin/nologin comenq || {
        echo "Failed to create user" >&2
        exit 1
    }
fi

chown comenq:comenq /var/lib/comenq
chmod 750 /var/lib/comenq
