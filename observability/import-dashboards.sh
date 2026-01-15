#!/bin/bash
# Script to import recommended Grafana dashboards
# Run this after Grafana is running

set -e

GRAFANA_URL="${GRAFANA_URL:-http://localhost:3050}"
GRAFANA_AUTH="${GRAFANA_AUTH:-admin:admin}"

# Our fixed datasource UIDs (must match datasources.yml)
PROMETHEUS_UID="prometheus"
LOKI_UID="loki"
TEMPO_UID="tempo"

# Wait for Grafana to be ready
echo "Waiting for Grafana to be ready..."
for i in {1..30}; do
    if curl -s -u "$GRAFANA_AUTH" "$GRAFANA_URL/api/health" | grep -q "ok"; then
        echo "Grafana is ready!"
        break
    fi
    if [ $i -eq 30 ]; then
        echo "Grafana not ready after 30 seconds, continuing anyway..."
    fi
    sleep 1
done

# Function to fix datasource references in dashboard JSON
fix_datasources() {
    jq --arg prom "$PROMETHEUS_UID" --arg loki "$LOKI_UID" --arg tempo "$TEMPO_UID" '
        # Recursive function to fix datasource references
        walk(
            if type == "object" and has("datasource") then
                if .datasource == null then
                    .
                elif (.datasource | type) == "string" then
                    # String datasource - convert to object format
                    if (.datasource | ascii_downcase | contains("prometheus")) or .datasource == "${DS_PROMETHEUS}" then
                        .datasource = {"type": "prometheus", "uid": $prom}
                    elif (.datasource | ascii_downcase | contains("loki")) or .datasource == "${DS_LOKI}" then
                        .datasource = {"type": "loki", "uid": $loki}
                    elif (.datasource | ascii_downcase | contains("tempo")) or .datasource == "${DS_TEMPO}" then
                        .datasource = {"type": "tempo", "uid": $tempo}
                    else
                        .
                    end
                elif (.datasource | type) == "object" then
                    # Object datasource - fix the uid
                    if .datasource.type == "prometheus" then
                        .datasource.uid = $prom
                    elif .datasource.type == "loki" then
                        .datasource.uid = $loki
                    elif .datasource.type == "tempo" then
                        .datasource.uid = $tempo
                    else
                        .
                    end
                else
                    .
                end
            else
                .
            end
        )
    '
}

# Function to import a dashboard from grafana.com
import_from_grafana_com() {
    local name="$1"
    local id="$2"
    
    echo "Importing $name (ID: $id)..."
    
    # Fetch dashboard JSON from grafana.com
    local dashboard_json
    dashboard_json=$(curl -s "https://grafana.com/api/dashboards/$id/revisions/latest/download" 2>/dev/null)
    
    if [ -z "$dashboard_json" ] || echo "$dashboard_json" | grep -q "error"; then
        echo "  ✗ Failed to fetch dashboard from grafana.com"
        return 1
    fi
    
    # Process and import
    local temp_file
    temp_file=$(mktemp)
    
    echo "$dashboard_json" | fix_datasources | jq 'del(.id)' > "$temp_file" 2>/dev/null
    
    if [ ! -s "$temp_file" ]; then
        echo "  ✗ Failed to process dashboard JSON"
        rm -f "$temp_file"
        return 1
    fi
    
    # Wrap and import
    local import_json
    import_json=$(jq '{dashboard: ., overwrite: true}' "$temp_file")
    rm -f "$temp_file"
    
    local response
    response=$(curl -s -X POST \
        -u "$GRAFANA_AUTH" \
        -H "Content-Type: application/json" \
        -d "$import_json" \
        "$GRAFANA_URL/api/dashboards/db" 2>&1)
    
    if echo "$response" | grep -q '"status":"success"\|"version"'; then
        echo "  ✓ $name imported successfully"
        return 0
    else
        echo "  ✗ Failed: $(echo "$response" | jq -r '.message // .status // .' 2>/dev/null | head -c 100)"
        return 1
    fi
}

# Function to import a local dashboard
import_local_dashboard() {
    local name="$1"
    local file="$2"
    
    echo "Importing $name..."
    
    if [ ! -f "$file" ]; then
        echo "  ✗ File not found: $file"
        return 1
    fi
    
    # Process and import
    local import_json
    import_json=$(cat "$file" | fix_datasources | jq '{dashboard: (. | del(.id)), overwrite: true}' 2>/dev/null)
    
    if [ -z "$import_json" ]; then
        echo "  ✗ Failed to process dashboard JSON"
        return 1
    fi
    
    local response
    response=$(curl -s -X POST \
        -u "$GRAFANA_AUTH" \
        -H "Content-Type: application/json" \
        -d "$import_json" \
        "$GRAFANA_URL/api/dashboards/db" 2>&1)
    
    if echo "$response" | grep -q '"status":"success"\|"version"'; then
        echo "  ✓ $name imported successfully"
        return 0
    else
        echo "  ✗ Failed: $(echo "$response" | jq -r '.message // .status // .' 2>/dev/null | head -c 100)"
        return 1
    fi
}

echo ""
echo "=== Importing dashboards from grafana.com ==="
echo ""

# Import recommended community dashboards
import_from_grafana_com "Node Exporter Full" "1860" || true
import_from_grafana_com "OpenTelemetry Collector" "15983" || true

echo ""
echo "=== Importing local MoQ dashboards ==="
echo ""

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Import local dashboards
import_local_dashboard "MoQ Pipeline" "$SCRIPT_DIR/grafana/dashboards/moq-pipeline.json" || true
import_local_dashboard "MoQ Overview" "$SCRIPT_DIR/grafana/dashboards/moq-overview.json" || true

echo ""
echo "=== Dashboard Import Complete ==="
echo ""
echo "Access Grafana at: $GRAFANA_URL"
echo "Default login: admin / admin"
echo ""
echo "Available dashboards:"
curl -s -u "$GRAFANA_AUTH" "$GRAFANA_URL/api/search?type=dash-db" 2>/dev/null | jq -r '.[].title' | sed 's/^/  - /'
echo ""
