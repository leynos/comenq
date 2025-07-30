#!/bin/bash
systemctl daemon-reload
systemctl enable comenqd.service
systemctl start comenqd.service
