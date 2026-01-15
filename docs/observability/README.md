# MoQ Observability Documentation

> **Main documentation is in [`/observability/README.md`](../../observability/README.md)**

This folder contains supplementary documentation for the MoQ observability stack.

## Quick Links

- **[Main README](../../observability/README.md)** - Architecture, setup, and usage
- **[Quick Start](./QUICKSTART.md)** - Get up and running in 5 minutes
- **[Metrics Reference](./metrics.md)** - All available metrics
- **[Implementation Status](./IMPLEMENTATION_STATUS.md)** - What's done, what's planned

## TL;DR

```bash
# Start observability stack
cd observability && docker compose up -d

# Import dashboards
./import-dashboards.sh

# Start relay
just dev

# Open Grafana
open http://localhost:3050
```

## Architecture Summary

```
┌─────────────┐    ┌─────────────┐
│   Browser   │    │  MoQ Relay  │
│   Player    │    │   (Rust)    │
└──────┬──────┘    └──────┬──────┘
       │ OTLP/HTTP        │ OTLP/gRPC
       │                  │
       ▼                  ▼
┌─────────────────────────────────┐
│      OpenTelemetry Collector    │
└──────┬──────────┬───────────────┘
       │          │          
       ▼          ▼          
┌──────────┐ ┌─────────┐ ┌──────┐
│Prometheus│ │  Tempo  │ │ Loki │
│ metrics  │ │ traces  │ │ logs │
└────┬─────┘ └────┬────┘ └──┬───┘
     │            │         │
     └────────────┼─────────┘
                  ▼
           ┌──────────┐
           │ Grafana  │
           │ :3050    │
           └──────────┘
```

## Key Metrics

| What | Metric | Where |
|------|--------|-------|
| Active viewers | `moq_relay_active_subscribers` | Relay |
| Buffer health | `moq_client_buffer_length_seconds` | Client |
| Startup time | `moq_client_startup_time_seconds` | Client |
| Transport type | `moq_client_connections_total{transport=...}` | Client |
| Bandwidth | `moq_relay_bytes_sent_total` | Relay |
