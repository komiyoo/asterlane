import { Badge, Button, Table } from "@cloudflare/kumo";
import { Fragment, useState } from "react";
import type {
  OutputHealthStatus,
  OutputMcpHealthResponse,
  OutputMcpServerDetailResponse,
  OutputMcpServerResponse,
} from "../../api/index.ts";
import { ConfirmDialog } from "../../components/confirm-dialog.tsx";
import { EmptyState } from "../../components/empty-state.tsx";
import { ErrorState } from "../../components/error-state.tsx";
import { FormMessage } from "../../components/form-message.tsx";
import { LoadingState } from "../../components/loading-state.tsx";
import { formatOptionalNumber } from "../format.ts";
import { effectiveDescription } from "../tools/catalog.ts";
import { ToolDebugPanel } from "../tools/debug-panel.tsx";
import { ToolMetadataPanel } from "../tools/metadata-panel.tsx";

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
  onRefreshDetail: (id: string) => void;
}) {
  const [debugName, setDebugName] = useState<string | null>(null);
  const [metaName, setMetaName] = useState<string | null>(null);

  if (servers.length === 0) {
    return <EmptyState title="无 MCP 服务" />;
  }

  return (
    <div className="tablewrap">
      <Table>
        <Table.Header variant="compact">
          <Table.Row>
            <Table.Head>状态</Table.Head>
            <Table.Head>ID</Table.Head>
            <Table.Head>领域 / 提供商</Table.Head>
            <Table.Head>URL</Table.Head>
            <Table.Head>需要 key</Table.Head>
            <Table.Head>工具数</Table.Head>
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
  onRefreshDetail: () => void;
  onDebug: (name: string) => void;
  onMeta: (name: string) => void;
}) {
  return (
    <>
      <Table.Row>
        <Table.Cell>
          <Badge variant={healthBadges[health.status]} appearance="dot">
            {healthLabels[health.status]}
          </Badge>
          {health.last_error === null ? null : <div className="err">{health.last_error}</div>}
        </Table.Cell>
        <Table.Cell>
          {server.id}
          {server.builtin ? (
            <>
              {" "}
              <Badge variant="neutral">内置</Badge>
            </>
          ) : null}
        </Table.Cell>
        <Table.Cell>
          {server.domain} / {server.provider}
        </Table.Cell>
        <Table.Cell>{server.url}</Table.Cell>
        <Table.Cell>{server.requires_key ? `是（${server.auth_type}）` : "否"}</Table.Cell>
        <Table.Cell>{server.tool_count}</Table.Cell>
        <Table.Cell>
          <div className="row-actions">
            <Button type="button" aria-expanded={open} onClick={onToggle}>
              详情
            </Button>
            <Button type="button" disabled={probing} onClick={onProbe}>
              {probing ? "探测中…" : "探测"}
            </Button>
            <Button type="button" onClick={onEdit}>
              编辑
            </Button>
            <ConfirmDialog
              label="删除"
              title="删除 MCP 服务"
              description="删除后，该服务的工具会从目录移除。"
              resource={server.id}
              confirmLabel="确认删除"
              onConfirm={onDelete}
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
  return (
    <div className="detail">
      <p>描述：{detail.description || "—"}</p>
      <p>
        健康：{healthLabels[health.status]}（{health.status}） · 末次检查{" "}
        {health.last_check_at ?? "—"} · 末次正常 {health.last_ok_at ?? "—"} · 延迟(ms){" "}
        {formatOptionalNumber(health.latency_ms) || "—"} · 连续失败 {health.consecutive_failures}
      </p>
      {health.last_error === null ? null : <p className="err">最近错误：{health.last_error}</p>}
      <p>测活：{detail.health_check_enabled ? "开启" : "关闭"}</p>
      <p>
        限额：rps {detail.limits.rps ?? "—"} · rpm {detail.limits.rpm ?? "—"} · 最大并发{" "}
        {detail.limits.max_concurrent ?? "—"}
      </p>
      <p>
        完整性策略：{detail.security.integrity_policy} · 防御：
        {detail.security.defense_enabled ? "开" : "关"} · 结果预算：
        {detail.security.result_budget_bytes ?? "—"}
      </p>
      <h3 className="section-title">工具（{detail.tools.length}）</h3>
      {detail.tools.length === 0 ? (
        <p>无工具</p>
      ) : (
        <div className="tablewrap">
          <Table>
            <Table.Header variant="compact">
              <Table.Row>
                <Table.Head>工具名</Table.Head>
                <Table.Head>描述（有效）</Table.Head>
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
                          <Badge variant="neutral">已覆盖</Badge>
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
                          介绍
                        </Button>
                        <Button type="button" onClick={() => onDebug(tool.wire_name)}>
                          调试
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

const healthLabels = {
  ok: "正常",
  unreachable: "不可达",
  auth_required: "需要授权",
  disabled: "已停用",
  unknown: "未知",
} as const satisfies Record<OutputHealthStatus, string>;
