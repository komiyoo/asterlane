import { Badge, Button, Link, Table } from "@cloudflare/kumo";
import type { OutputMcpPresetResponse } from "../../api/index.ts";
import { ErrorState } from "../../components/error-state.tsx";
import { FormMessage } from "../../components/form-message.tsx";
import { LoadingState } from "../../components/loading-state.tsx";

export function PresetList({
  presets,
  error,
  loading,
  notice,
  enablingId,
  onEnable,
  onConfigure,
  onRetry,
}: {
  presets: OutputMcpPresetResponse[] | null;
  error: unknown;
  loading: boolean;
  notice: string | null;
  enablingId: string | null;
  onEnable: (preset: OutputMcpPresetResponse) => void;
  onConfigure: (preset: OutputMcpPresetResponse) => void;
  onRetry: () => void;
}) {
  return (
    <section aria-label="内置集成">
      <h3 className="section-title">内置集成</h3>
      {notice === null ? null : <FormMessage tone="error">{notice}</FormMessage>}
      {error !== null ? <ErrorState error={error} onRetry={onRetry} /> : null}
      {loading && presets === null ? <LoadingState /> : null}
      {presets === null ? null : (
        <div className="tablewrap">
          <Table>
            <Table.Header variant="compact">
              <Table.Row>
                <Table.Head>集成</Table.Head>
                <Table.Head>描述</Table.Head>
                <Table.Head>状态</Table.Head>
                <Table.Head>凭据</Table.Head>
                <Table.Head></Table.Head>
              </Table.Row>
            </Table.Header>
            <Table.Body>
              {presets.length === 0 ? (
                <Table.Row>
                  <Table.Cell colSpan={5}>无内置集成</Table.Cell>
                </Table.Row>
              ) : (
                presets.map((preset) => (
                  <Table.Row key={preset.id}>
                    <Table.Cell>
                      {preset.id}
                      <div className="hint">
                        {preset.domain} / {preset.provider}
                      </div>
                    </Table.Cell>
                    <Table.Cell className="wrap">{preset.description}</Table.Cell>
                    <Table.Cell>
                      <Badge variant={preset.enabled ? "success" : "neutral"} appearance="dot">
                        {preset.enabled ? "已启用" : "未启用"}
                      </Badge>
                    </Table.Cell>
                    <Table.Cell>
                      {preset.requires_key ? "需 key" : "免费"}
                      {safeApplyUrl(preset.apply_url) === null ? null : (
                        <>
                          {" "}
                          <Link
                            href={safeApplyUrl(preset.apply_url) ?? undefined}
                            target="_blank"
                            rel="noreferrer"
                          >
                            申请 key
                            <Link.ExternalIcon />
                          </Link>
                        </>
                      )}
                    </Table.Cell>
                    <Table.Cell>
                      {preset.enabled ? (
                        <Button type="button" disabled>
                          已启用
                        </Button>
                      ) : preset.requires_key ? (
                        <Button type="button" onClick={() => onConfigure(preset)}>
                          配置 key 启用
                        </Button>
                      ) : (
                        <Button
                          type="button"
                          disabled={enablingId === preset.id}
                          onClick={() => onEnable(preset)}
                        >
                          {enablingId === preset.id ? "启用中…" : "启用"}
                        </Button>
                      )}
                    </Table.Cell>
                  </Table.Row>
                ))
              )}
            </Table.Body>
          </Table>
        </div>
      )}
    </section>
  );
}

function safeApplyUrl(value: string | null): string | null {
  if (value === null) {
    return null;
  }
  if (value.startsWith("https://") || value.startsWith("http://")) {
    return value;
  }
  return null;
}
