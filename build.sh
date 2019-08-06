#!/bin/bash
set -e
set -x

echo "Running build for $REPO_NAME"
# GCS w/ sccache currently does not work :/
# export SCCACHE_GCS_BUCKET=umpyre-sccache
# export SCCACHE_GCS_RW_MODE=READ_WRITE
export SCCACHE_GCS_KEY_PATH=/root/sccache.json
export SCCACHE_DIR=/workspace/sccache
mkdir -p $SCCACHE_DIR

mkdir -p $HOME/.ssh
chmod 0700 $HOME/.ssh
ssh-keyscan github.com > $HOME/.ssh/known_hosts

# Don't echo secrets
set +x
echo "$SSH_KEY" > $HOME/.ssh/id_rsa
echo "$SCCACHE_KEY" > $SCCACHE_GCS_KEY_PATH
set -x

chmod 600 $HOME/.ssh/id_rsa
eval `ssh-agent`
ssh-add -k $HOME/.ssh/id_rsa

gcloud auth activate-service-account --key-file=$SCCACHE_GCS_KEY_PATH
gsutil -m -q rsync -r gs://umpyre-sccache/sccache $SCCACHE_DIR || true
mkdir -p target
gsutil -m -q rsync -r gs://umpyre-sccache/$REPO_NAME/target target || true

sccache -s

yarn install
cargo build --release --out-dir=out -Z unstable-options

sccache -s

gsutil -m -q rsync -r $SCCACHE_DIR gs://umpyre-sccache/sccache || true
gsutil -m -q rsync -r target gs://umpyre-sccache/$REPO_NAME/target || true
