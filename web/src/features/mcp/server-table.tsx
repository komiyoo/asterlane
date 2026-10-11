import { Badge, Button, Table } from "@cloudflare/kumo";
import { Fragment, useState } from "react";
import type {
  OutputHealthStatus,
  OutputMcpHealthResponse,
  OutputMcpOAuthResponse,
  OutputMcpServerDetailResponse,
  OutputMcpServerResponse,
} from "../../api/index.ts";
import { ConfirmDialog } from "../../components/confirm-dialog.tsx";
import { EmptyState } from "../../components/empty-state.tsx";
import { ErrorState } from "../../components/error-state.tsx";
import { FormMessage } from "../../components/form-message.tsx";
import { LoadingState } from "../../components/loading-state.tsx";
import { text } from "../../i18n/locale.ts";
import { formatCount, formatOptionalNumber, formatTimestamp } from "../format.ts";
import { effectiveDescription } from "../tools/catalog.ts";
import { ToolDebugPanel } from "../tools/debug-panel.tsx";
import { ToolMetadataPanel } from "../tools/metadata-panel.tsx";
import { OAuthActions } from "./oauth-actions.tsx";
import { oauthStatusText } from "./write.ts";

export type DetailState =
  | { status: "loading" }
  | { status: "ready"; detail: OutputMcpServerDetailResponse }
  | { status: "error"; error: unknown };

export function ServerTable({
  servers,
  openIds,
  details,
  healthOverlay,
  probingId,
  probeErrors,
  onToggle,
  onProbe,
  onEdit,
  onDelete,
  onDeauthorize,
  onRefreshDetail,
}: {
  servers: OutputMcpServerResponse[];
  openIds: readonly string[];
  details: Readonly<Record<string, DetailState>>;
  healthOverlay: Readonly<Record<string, OutputMcpHealthResponse>>;
  probingId: string | null;
  probeErrors: Readonly<Record<string, string>>;
  onToggle: (id: string) => void;
  onProbe: (id: string) => void;
  onEdit: (server: OutputMcpServerResponse) => void;
  onDelete: (id: string) => void;
  onDeauthorize: (id: string) => void;
  onRefreshDetail: (id: string) => void;
}) {
  const [debugName, setDebugName] = useState<string | null>(null);
  const [metaName, setMetaName] = useState<string | null>(null);

  if (servers.length === 0) {
    return <EmptyState title={text("mcp.table.empty")} />;
  }

  return (
    <div className="tablewrap">
      <Table>
        <Table.Header variant="compact">
          <Table.Row>
            <Table.Head>{text("mcp.table.status")}</Table.Head>
            <Table.Head>ID</Table.Head>
            <Table.Head>{text("mcp.table.domain")}</Table.Head>
            <Table.Head>URL</Table.Head>
            <Table.Head>{text("mcp.table.needsKey")}</Table.Head>
            <Table.Head>{text("mcp.table.tools")}</Table.Head>
            <Table.Head></Table.Head>
          </Table.Row>
        </Table.Header>
        <Table.Body>
          {servers.map((server) => {
            const open = openIds.includes(server.id);
            const health = healthOverlay[server.id] ?? server.health;
            const detail = details[server.id];
            return (
              <ServerRows
                key={server.id}
                server={server}
                open={open}
                health={health}
                detail={detail}
                probing={probingId === server.id}
                probeError={probeErrors[server.id] ?? null}
                debugName={debugName}
                metaName={metaName}
                onToggle={() => onToggle(server.id)}
                onProbe={() => onProbe(server.id)}
                onEdit={() => onEdit(server)}
                onDelete={() => onDelete(server.id)}
                onDeauthorize={onDeauthorize}
                onRefreshDetail={() => onRefreshDetail(server.id)}
                onDebug={(name) => setDebugName((current) => (current === name ? null : name))}
                onMeta={(name) => setMetaName((current) => (current === name ? null : name))}
              />
            );
          })}
        </Table.Body>
      </Table>
    </div>
  );
}

function ServerRows({
  server,
  open,
  health,
  detail,
  probing,
  probeError,
  debugName,
  metaName,
  onToggle,
  onProbe,
  onEdit,
  onDelete,
  onDeauthorize,
  onRefreshDetail,
  onDebug,
  onMeta,
}: {
  server: OutputMcpServerResponse;
  open: boolean;
  health: OutputMcpHealthResponse;
  detail: DetailState | undefined;
  probing: boolean;
  probeError: string | null;
  debugName: string | null;
  metaName: string | null;
  onToggle: () => void;
  onProbe: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onDeauthorize: (id: string) => void;
  onRefreshDetail: () => void;
  onDebug: (name: string) => void;
  onMeta: (name: string) => void;
}) {
  return (
    <>
      <Table.Row>
        <Table.Cell>
          <Badge variant={healthBadges[health.status]} appearance="dot">
            {healthLabel(health.status)}
          </Badge>
          {health.last_error === null ? null : <div className="err">{health.last_error}</div>}
        </Table.Cell>
        <Table.Cell>
          {server.id}
          {server.builtin ? (
            <>
              {" "}
              <Badge variant="neutral">{text("mcp.table.builtin")}</Badge>
            </>
          ) : null}
        </Table.Cell>
        <Table.Cell>
          {server.domain} / {server.provider}
        </Table.Cell>
        <Table.Cell>{server.url}</Table.Cell>
        <Table.Cell>
          {server.requires_key
            ? text("mcp.table.keyed", { auth: server.auth_type })
            : text("common.no")}
        </Table.Cell>
        <Table.Cell>{formatCount(server.tool_count)}</Table.Cell>
        <Table.Cell>
          <div className="row-actions">
            <Button type="button" aria-expanded={open} onClick={onToggle}>
              {text("mcp.table.details")}
            </Button>
            <Button type="button" disabled={probing} onClick={onProbe}>
              {probing ? text("mcp.table.probing") : text("mcp.table.probe")}
            </Button>
            <Button type="button" onClick={onEdit}>
              {text("mcp.table.edit")}
            </Button>
            <ConfirmDialog
              label={text("mcp.table.delete")}
              title={text("mcp.table.deleteTitle")}
              description={text("mcp.table.deleteDescription")}
              resource={server.id}
              confirmLabel={text("mcp.table.deleteConfirm")}
              onConfirm={onDelete}
            />
            <OAuthActions
              key={`${server.oauth?.grant ?? ""}:${server.oauth?.status ?? ""}`}
              server={server}
              onDeauthorize={onDeauthorize}
            />
          </div>
          {probeError === null ? null : <FormMessage tone="error">{probeError}</FormMessage>}
        </Table.Cell>
      </Table.Row>
      {open ? (
        <Table.Row>
          <Table.Cell colSpan={7}>
            {detail === undefined || detail.status === "loading" ? <LoadingState /> : null}
            {detail?.status === "error" ? (
              <ErrorState error={detail.error} onRetry={onRefreshDetail} />
            ) : null}
            {detail?.status === "ready" ? (
              <ServerDetail
                detail={detail.detail}
                health={health}
                debugName={debugName}
                metaName={metaName}
                onDebug={onDebug}
                onMeta={onMeta}
                onSaved={onRefreshDetail}
              />
            ) : null}
          </Table.Cell>
        </Table.Row>
      ) : null}
    </>
  );
}

function ServerDetail({
  detail,
  health,
  debugName,
  metaName,
  onDebug,
  onMeta,
  onSaved,
}: {
  detail: OutputMcpServerDetailResponse;
  health: OutputMcpHealthResponse;
  debugName: string | null;
  metaName: string | null;
  onDebug: (name: string) => void;
  onMeta: (name: string) => void;
  onSaved: () => void;
}) {
  const dash = text("common.dash");
  return (
    <div className="detail">
      <p>{text("mcp.detail.description", { text: detail.description || dash })}</p>
      <p>
        {text("mcp.detail.health", {
          label: healthLabel(health.status),
          status: health.status,
          checked: formatTimestamp(health.last_check_at) || dash,
          okAt: formatTimestamp(health.last_ok_at) || dash,
          latency: formatOptionalNumber(health.latency_ms) || dash,
          failures: formatCount(health.consecutive_failures),
        })}
      </p>
      {health.last_error === null ? null : (
        <p className="err">{text("mcp.detail.lastError", { error: health.last_error })}</p>
      )}
      <p>
        {detail.health_check_enabled ? text("mcp.detail.probeOn") : text("mcp.detail.probeOff")}
      </p>
      {detail.oauth === null ? null : <p>{oauthDetailText(detail.oauth)}</p>}
      <p>
        {text("mcp.detail.limits", {
          rps: formatOptionalNumber(detail.limits.rps) || dash,
          rpm: formatOptionalNumber(detail.limits.rpm) || dash,
          concurrent: formatOptionalNumber(detail.limits.max_concurrent) || dash,
        })}
      </p>
      <p>
        {text("mcp.detail.integrity", { policy: detail.security.integrity_policy })}
        {" · "}
        {text("mcp.detail.defense", {
          state: detail.security.defense_enabled ? text("common.on") : text("common.off"),
        })}
        {" · "}
        {text("mcp.detail.budget", {
          bytes: formatOptionalNumber(detail.security.result_budget_bytes) || dash,
        })}
      </p>
      <h3 className="section-title">
        {text("mcp.detail.tools", { count: formatCount(detail.tools.length) })}
      </h3>
      {detail.tools.length === 0 ? (
        <p>{text("mcp.detail.noTools")}</p>
      ) : (
        <div className="tablewrap">
          <Table>
            <Table.Header variant="compact">
              <Table.Row>
                <Table.Head>{text("mcp.detail.toolName")}</Table.Head>
                <Table.Head>{text("mcp.detail.effectiveDescription")}</Table.Head>
                <Table.Head></Table.Head>
              </Table.Row>
            </Table.Header>
            <Table.Body>
              {detail.tools.map((tool) => (
                <Fragment key={tool.wire_name}>
                  <Table.Row>
                    <Table.Cell>
                      <span className="mono">{tool.wire_name}</span>
                      {tool.description_override === null ? null : (
                        <>
                          {" "}
                          <Badge variant="neutral">{text("mcp.detail.overridden")}</Badge>
                        </>
                      )}
                    </Table.Cell>
                    <Table.Cell className="wrap">
                      {effectiveDescription({
                        name: tool.wire_name,
                        resource_id: detail.id,
                        description: tool.description,
                        description_override: tool.description_override,
                      })}
                    </Table.Cell>
                    <Table.Cell>
                      <div className="row-actions">
                        <Button type="button" onClick={() => onMeta(tool.wire_name)}>
                          {text("mcp.detail.intro")}
                        </Button>
                        <Button type="button" onClick={() => onDebug(tool.wire_name)}>
                          {text("mcp.detail.debug")}
                        </Button>
                      </div>
                    </Table.Cell>
                  </Table.Row>
                  {metaName === tool.wire_name ? (
                    <Table.Row>
                      <Table.Cell colSpan={3}>
                        <ToolMetadataPanel
                          toolName={tool.wire_name}
                          original={tool.description}
                          initialOverride={tool.description_override ?? ""}
                          onSaved={onSaved}
                        />
                      </Table.Cell>
                    </Table.Row>
                  ) : null}
                  {debugName === tool.wire_name ? (
                    <Table.Row>
                      <Table.Cell colSpan={3}>
                        <ToolDebugPanel toolName={tool.wire_name} />
                      </Table.Cell>
                    </Table.Row>
                  ) : null}
                </Fragment>
              ))}
            </Table.Body>
          </Table>
        </div>
      )}
    </div>
  );
}

const healthBadges = {
  ok: "success",
  unreachable: "error",
  auth_required: "warning",
  disabled: "neutral",
  unknown: "neutral",
} as const satisfies Record<OutputHealthStatus, string>;

function healthLabel(status: string): string {
  switch (status) {
    case "ok":
      return text("mcp.health.ok");
    case "unreachable":
      return text("mcp.health.unreachable");
    case "auth_required":
      return text("mcp.health.authRequired");
    case "disabled":
      return text("mcp.health.disabled");
    case "unknown":
      return text("mcp.health.unknown");
    default:
      return text("mcp.health.unknown");
  }
}

function oauthDetailText(oauth: OutputMcpOAuthResponse): string {
  const line = text("mcp.detail.oauth", {
    status: oauthStatusText(oauth.status),
    code: oauth.status,
    grant: oauth.grant,
    expires: formatTimestamp(oauth.expires_at) || text("common.dash"),
  });
  if (oauth.expires_at === undefined || oauth.expires_at === null || oauth.expires_at === "") {
    return line;
  }
  return line + text("mcp.detail.oauthRefresh");
}
