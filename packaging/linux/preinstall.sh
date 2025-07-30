#!/bin/bash
if ! getent group comenq >/dev/null; then
    groupadd --system comenq
fi
if ! getent passwd comenq >/dev/null; then
    useradd --system --gid comenq --home-dir /var/lib/comenq --create-home --shell /sbin/nologin comenq
fi
