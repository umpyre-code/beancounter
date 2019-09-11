#!/bin/sh

set -e
set -x

envsubst < /etc/config/BeanCounter.toml.in > BeanCounter.toml

exec "$@"
