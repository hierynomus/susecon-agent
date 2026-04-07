# SUSECon Agent — Kubernetes Swap Demo

A deliberately memory-hungry MCP server that recommends SUSECon sessions.
Built to demonstrate why Kubernetes swap support matters for AI/agent workloads.

**The pitch:** AI agents are bursty by nature. You cannot predict how much memory
a chain-of-thought with multiple tool calls will need. Traditional Kubernetes
says: set your limits high and waste resources, or set them tight and get killed.
Swap gives you a third option — a pressure valve.

## How It Works

1. An LLM connects to the SUSECon Agent via MCP (Streamable HTTP transport)
2. User asks: *"What sessions should I attend about AI?"*
3. The LLM calls the `recommend_sessions` tool
4. The agent **gradually allocates ~512 MB of memory** over 12 seconds
   (simulating context accumulation from multi-tool reasoning)
5. The pod has a 256 Mi memory limit:
   - **Without swap:** Kubernetes OOM-kills the pod mid-call → the user gets nothing
   - **With swap:** excess memory pages to swap, the agent completes → the user gets session recommendations

## Architecture

```
┌──────────┐      MCP/HTTP       ┌──────────────────┐     ConfigMap
│   LLM    │ ──────────────────▶ │  susecon-agent   │ ◀── sessions.yaml
│ (Claude) │  POST /mcp          │  (Rust + rmcp)   │
└──────────┘                     │                  │
                                 │  recommend_      │
                                 │  sessions()      │
                                 │    ↓             │
                                 │  allocate memory │
                                 │  (gradual, 12s)  │
                                 │    ↓             │
                                 │  search & return │
                                 └──────────────────┘
```

## Prerequisites

- RKE2 cluster (v1.32+ recommended, see swap setup below)
- Helm 3
- Container image built and available (see Building below)

## Building

```bash
# Build the container image
docker build -t ghcr.io/suse/susecon-agent:0.1.0 .

# Push to your registry
docker push ghcr.io/suse/susecon-agent:0.1.0
```

## Deploying with Helm

```bash
# Install without swap (demo: watch it crash)
helm install susecon-agent ./chart/susecon-agent \
  --namespace susecon-demo \
  --create-namespace

# Watch the pod
kubectl -n susecon-demo get pods -w

# Trigger the tool call (via your MCP client or curl)
# The pod will OOM-kill after ~8-10 seconds of memory growth
```

### Customizing the Demo

Override values for different demo scenarios:

```bash
# Bigger bloat, longer duration (more dramatic)
helm upgrade susecon-agent ./chart/susecon-agent \
  --namespace susecon-demo \
  --set memoryBloat.megabytes=768 \
  --set memoryBloat.durationSeconds=15

# Tighter limit (faster crash)
helm upgrade susecon-agent ./chart/susecon-agent \
  --namespace susecon-demo \
  --set resources.limits.memory=128Mi

# Custom session data
helm upgrade susecon-agent ./chart/susecon-agent \
  --namespace susecon-demo \
  --set sessionData.create=false \
  --set sessionData.existingConfigMap=my-sessions
```

---

# RKE2 Swap Setup Guide

## Overview

Kubernetes swap support requires three things:

1. **Swap space on the node** (file, partition, or zram)
2. **Kubelet configured** with `failSwapOn: false` and `memorySwap.swapBehavior: LimitedSwap`
3. **cgroup v2** (required — cgroup v1 nodes can only use `NoSwap`)

RKE2 already ships with `--fail-swap-on=false` by default, which means the
kubelet won't crash if swap is detected. But we still need to explicitly enable
the `LimitedSwap` behavior so pods can actually *use* swap.

## Step 1: Verify cgroup v2

```bash
stat -fc %T /sys/fs/cgroup
```

Expected output: `cgroup2fs`

If you see `tmpfs`, your system uses cgroup v1 and swap support for containers
will not work. You need to boot with `systemd.unified_cgroup_hierarchy=1` in
your kernel parameters.

## Step 2: Create Swap Space

You have three options. **None of them require a dedicated swap partition.**

### Option A: Swap File (recommended for demos)

The simplest approach. Creates a file on the existing filesystem and uses it as
swap. Works everywhere, no repartitioning needed.

```bash
# Create a 2 GB swap file
sudo fallocate -l 2G /swapfile
sudo chmod 600 /swapfile
sudo mkswap /swapfile
sudo swapon /swapfile

# Make it persistent across reboots
echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab

# Verify
swapon --show
free -h
```

### Option B: zram (no disk needed at all)

zram creates a compressed swap device **entirely in RAM**. No disk I/O, no
partition, no file. The tradeoff: it uses CPU for compression, and total
capacity is bounded by physical RAM.

Perfect for demos on machines where you don't want to touch the disk.

```bash
# Load the zram module
sudo modprobe zram num_devices=1

# Set compression algorithm and size (1 GB, ~2-3x effective with compression)
echo lz4 | sudo tee /sys/block/zram0/comp_algorithm
echo 1G | sudo tee /sys/block/zram0/disksize

# Format and enable
sudo mkswap /dev/zram0
sudo swapon -p 100 /dev/zram0

# Verify
swapon --show
```

To make zram persistent, use systemd's `zram-generator`:

```bash
# Install zram-generator (SLE/openSUSE)
sudo zypper install systemd-zram-service

# Or create the config manually
sudo mkdir -p /etc/systemd/zram-generator.conf.d
cat <<EOF | sudo tee /etc/systemd/zram-generator.conf.d/susecon-demo.conf
[zram0]
zram-size = ram / 2
compression-algorithm = lz4
swap-priority = 100
EOF

# Enable and start
sudo systemctl daemon-reload
sudo systemctl start /dev/zram0
```

### Option C: Swap Partition (traditional)

If your node already has an unused partition:

```bash
sudo mkswap /dev/sdX
sudo swapon /dev/sdX
echo '/dev/sdX none swap sw 0 0' | sudo tee -a /etc/fstab
```

## Step 3: Configure RKE2 Kubelet for Swap

RKE2 v1.32+ supports kubelet configuration via drop-in files. Create a
drop-in config that enables `LimitedSwap`:

```bash
sudo mkdir -p /var/lib/rancher/rke2/agent/etc/kubelet.conf.d

cat <<EOF | sudo tee /var/lib/rancher/rke2/agent/etc/kubelet.conf.d/10-swap.conf
apiVersion: kubelet.config.k8s.io/v1beta1
kind: KubeletConfiguration
failSwapOn: false
memorySwap:
  swapBehavior: LimitedSwap
EOF
```

For RKE2 versions below v1.32, use kubelet-arg in the RKE2 config instead:

```bash
# /etc/rancher/rke2/config.yaml
kubelet-arg:
  - "fail-swap-on=false"
```

And create a kubelet config file:

```bash
cat <<EOF | sudo tee /etc/rancher/rke2/kubelet-swap.yaml
apiVersion: kubelet.config.k8s.io/v1beta1
kind: KubeletConfiguration
failSwapOn: false
memorySwap:
  swapBehavior: LimitedSwap
EOF
```

Then reference it:

```yaml
# /etc/rancher/rke2/config.yaml
kubelet-arg:
  - "config=/etc/rancher/rke2/kubelet-swap.yaml"
```

## Step 4: Restart RKE2

```bash
sudo systemctl restart rke2-server   # or rke2-agent for worker nodes
```

## Step 5: Verify Swap Is Active

```bash
# Check node-level swap
ssh <node> "swapon --show && free -h"

# Check kubelet config via API
kubectl get --raw "/api/v1/nodes/<node-name>/proxy/configz" | jq '.kubeletconfig.memorySwap'
# Expected: {"swapBehavior":"LimitedSwap"}

# Check for the Swap condition on the node (Kubernetes 1.32+)
kubectl get node <node-name> -o jsonpath='{.status.conditions[?(@.type=="Swap")]}' | jq .
```

## Important: Swap Behavior Explained

With `LimitedSwap`, a Burstable pod's swap allowance is proportional to its
memory request relative to total node memory:

```
containerSwapLimit = (containerMemoryRequest / nodeTotalMemory) × totalPodsSwapAvailable
```

Only **Burstable** QoS pods can use swap. Guaranteed and BestEffort pods cannot.
Our demo pod is Burstable because it has both requests and limits set, with
limits > requests.

---

# Demo Runbook

## The Narrative Setup

Before diving into the demo, establish the constraints with the audience:

```bash
# Show the namespace resource quota
kubectl -n susecon-demo describe resourcequota
# Output shows:
#   limits.memory:    512Mi (total namespace budget)
#   requests.memory:  512Mi

# Show the per-container limit range
kubectl -n susecon-demo describe limitrange
# Output shows:
#   Max memory per container: 256Mi
#   Default memory limit:     256Mi
```

**The talking point:**

> *"Our platform team has set a memory budget on this namespace: 512 Mi total,
> 256 Mi max per container. That's sensible governance — we're sharing this
> cluster with dozens of other teams. But our AI agent is bursty. When it
> reasons across multiple tools, context accumulates and memory spikes to
> 400-500 MB. We can't just raise the limit — the namespace quota won't
> allow it. And even if we could, we'd be wasting memory 95% of the time
> for a spike that lasts 12 seconds. We need a smarter solution."*

You can prove the point live:

```bash
# Try to deploy with a higher memory limit — Kubernetes rejects it
kubectl -n susecon-demo run test-pod --image=busybox \
  --overrides='{"spec":{"containers":[{"name":"test","image":"busybox","resources":{"limits":{"memory":"512Mi"}}}]}}' \
  --command -- sleep 3600
# Error: forbidden: exceeded quota: ... limits.memory=512Mi, requested: 512Mi
# (leaves no room for the already-running agent pod)
```

## Pre-staging (before the keynote)

Before the demo, ensure the kubelet on your demo node is already configured
with `LimitedSwap`. This avoids any kubelet restart during the live demo.

```bash
# On the demo node (or via cloud-init / image build):
sudo mkdir -p /var/lib/rancher/rke2/agent/etc/kubelet.conf.d
cat <<EOF | sudo tee /var/lib/rancher/rke2/agent/etc/kubelet.conf.d/10-swap.conf
apiVersion: kubelet.config.k8s.io/v1beta1
kind: KubeletConfiguration
failSwapOn: false
memorySwap:
  swapBehavior: LimitedSwap
EOF
sudo systemctl restart rke2-agent  # one-time, done before demo day
```

Also edit the Job files to set your node name:

```bash
# Replace placeholder in both job files
sed -i 's/<YOUR-SWAP-NODE-NAME>/my-worker-01/' jobs/enable-swap-job.yaml
sed -i 's/<YOUR-SWAP-NODE-NAME>/my-worker-01/' jobs/disable-swap-job.yaml
```

## Phase 1: The Crash (swap off)

```bash
# 1. Ensure swap is OFF (in case a previous demo run left it on)
kubectl delete job disable-swap -n kube-system --ignore-not-found
kubectl apply -f jobs/disable-swap-job.yaml
kubectl -n kube-system wait --for=condition=complete job/disable-swap --timeout=30s
kubectl -n kube-system logs job/disable-swap

# 2. Deploy the agent
helm install susecon-agent ./chart/susecon-agent \
  --namespace susecon-demo --create-namespace

# 3. Open three terminals:
#    Terminal 1: watch pods
kubectl -n susecon-demo get pods -w

#    Terminal 2: watch pod logs (memory growth)
kubectl -n susecon-demo logs -f deploy/susecon-agent

#    Terminal 3: watch events
kubectl -n susecon-demo get events -w --field-selector reason=OOMKilling

# 4. Trigger the tool call from your LLM
#    Ask: "What SUSECon sessions about AI would you recommend?"
#    Watch Terminal 2: RSS grows from ~20 MB -> 256 MB -> OOM killed
#    Watch Terminal 1: pod goes to OOMKilled status
#    The LLM gets no response.
```

## Phase 2: The Save (swap on)

```bash
# 1. Enable swap using the Job (no SSH needed!)
kubectl delete job enable-swap -n kube-system --ignore-not-found
kubectl apply -f jobs/enable-swap-job.yaml
kubectl -n kube-system wait --for=condition=complete job/enable-swap --timeout=60s
kubectl -n kube-system logs job/enable-swap
# Output shows: "=== Swap is now active ===" with swapon and free output

# 2. Restart the agent pod (to get a fresh process with no accumulated memory)
kubectl -n susecon-demo delete pod -l app.kubernetes.io/name=susecon-agent
kubectl -n susecon-demo rollout status deploy/susecon-agent

# 3. Open the same three terminals
# 4. Trigger the same tool call
#    Watch Terminal 2: RSS grows, but the pod SURVIVES
#    The response comes back after ~12 seconds
#    The LLM presents the session recommendations to the audience

# 5. (Optional) Show swap usage from another Job or kubectl debug
kubectl debug node/<node-name> -it --image=busybox -- sh -c "free -h && swapon --show"
```

## Phase 3: Cleanup

```bash
helm uninstall susecon-agent --namespace susecon-demo
kubectl delete namespace susecon-demo
```

---

# MCP Client Configuration

To connect your LLM to this MCP server, configure it as a remote MCP server
with Streamable HTTP transport:

```json
{
  "mcpServers": {
    "susecon-agent": {
      "url": "http://susecon-agent.susecon-demo.svc.cluster.local:8080/mcp",
      "transport": "streamable-http"
    }
  }
}
```

Or if port-forwarding for local testing:

```bash
kubectl -n susecon-demo port-forward svc/susecon-agent 8080:8080
```

Then use `http://localhost:8080/mcp` as the MCP server URL.
