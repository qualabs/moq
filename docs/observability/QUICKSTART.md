# MoQ Observability Quick Start

Get the observability stack running in 5 minutes.

## Prerequisites

- Docker with Compose plugin (`docker compose`)
- The MoQ project cloned and buildable

## Step 1: Start the Observability Stack

```bash
cd observability
docker compose up -d
```

This starts:
- **Prometheus** (metrics) - port 9090
- **Tempo** (traces) - port 3200
- **Loki** (logs) - port 3100
- **Grafana** (dashboards) - port 3050
- **OTel Collector** (telemetry ingestion) - ports 4317, 4318
- **Node Exporter** (system metrics) - port 9100
- **Alloy** (log collection)

Verify everything is running:
```bash
docker compose ps
```

## Step 2: Import Dashboards

```bash
./import-dashboards.sh
```

This imports:
- **MoQ Overview** - Main operational dashboard
- **MoQ Pipeline** - Detailed technical metrics
- **Node Exporter Full** - System metrics
- **OpenTelemetry Collector** - Collector health

## Step 3: Start the MoQ Relay

From the project root:
```bash
just dev
```

Or manually:
```bash
cd rs && cargo run --bin moq-relay
```

## Step 4: Open Grafana

Navigate to: **http://localhost:3050**

Login: `admin` / `admin`

Go to **Dashboards** → **MoQ Overview**

## Step 5: Generate Traffic

Open a browser tab to play video:
```
http://localhost:5173
```

Watch the dashboards update with:
- Active viewers count
- Buffer health metrics
- Connection types (WebTransport vs WebSocket)
- Bandwidth usage

## Verify Data is Flowing

### Check Prometheus Metrics

```bash
# List all MoQ metrics
curl -s 'http://localhost:9090/api/v1/label/__name__/values' | jq -r '.data[]' | grep moq

# Check active subscribers
curl -s 'http://localhost:9090/api/v1/query?query=moq_relay_active_subscribers' | jq '.data.result'

# Check client connections by transport
curl -s 'http://localhost:9090/api/v1/query?query=moq_client_connections_total' | jq '.data.result'
```

### Check OTel Collector

```bash
docker logs observability-otel-collector-1 2>&1 | tail -20
```

### Check Browser Console

Open browser DevTools (F12) and look for:
```
[Observability] Initialized with endpoint: http://localhost:4318
[Observability] Metrics will export every 10s
[Observability] Connection recorded: websocket
```

## Common Issues

### "No data" in dashboards

1. Check time range (top-right) is set to "Last 15 minutes"
2. Verify relay is running: `curl http://localhost:4443/health`
3. Check Prometheus targets: http://localhost:9090/targets

### CORS errors in browser console

The OTel Collector should handle CORS automatically. If you see errors:
1. Restart the collector: `docker compose restart otel-collector`
2. Check config: `cat otel-collector-config.yaml`

### Port conflicts

If port 3050 is busy, edit `docker-compose.yml`:
```yaml
grafana:
  ports:
    - "3051:3000"  # Change 3050 to another port
```

## Next Steps

- **Explore traces**: Grafana → Explore → Tempo
- **View logs**: Grafana → Explore → Loki
- **Create alerts**: Grafana → Alerting → Alert rules
- **Customize dashboards**: Clone and edit the MoQ dashboards

## Stopping the Stack

```bash
cd observability
docker compose down
```

To also remove data volumes:
```bash
docker compose down -v
```
