import { Badge, Button, Link, Table } from "@cloudflare/kumo";
import type { OutputMcpPresetResponse } from "../../api/index.ts";
import { ErrorState } from "../../components/error-state.tsx";
import { FormMessage } from "../../components/form-message.tsx";
import { LoadingState } from "../../components/loading-state.tsx";
import { text } from "../../i18n/locale.ts";

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
    <section aria-label={text("mcp.presets.region")}>
      <h3 className="section-title">{text("mcp.presets.region")}</h3>
      {notice === null ? null : <FormMessage tone="error">{notice}</FormMessage>}
      {error !== null ? <ErrorState error={error} onRetry={onRetry} /> : null}
      {loading && presets === null ? <LoadingState /> : null}
      {presets === null ? null : (
        <div className="tablewrap">
          <Table>
            <Table.Header variant="compact">
              <Table.Row>
                <Table.Head>{text("mcp.presets.integration")}</Table.Head>
                <Table.Head>{text("mcp.presets.description")}</Table.Head>
                <Table.Head>{text("mcp.presets.status")}</Table.Head>
                <Table.Head>{text("mcp.presets.credential")}</Table.Head>
                <Table.Head></Table.Head>
              </Table.Row>
            </Table.Header>
            <Table.Body>
              {presets.length === 0 ? (
                <Table.Row>
                  <Table.Cell colSpan={5}>{text("mcp.presets.empty")}</Table.Cell>
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
                        {preset.enabled
                          ? text("mcp.presets.enabled")
                          : text("mcp.presets.disabled")}
                      </Badge>
                    </Table.Cell>
                    <Table.Cell>
                      {preset.requires_key
                        ? text("mcp.presets.needsKey")
                        : text("mcp.presets.free")}
                      {safeApplyUrl(preset.apply_url) === null ? null : (
                        <>
                          {" "}
                          <Link
                            href={safeApplyUrl(preset.apply_url) ?? undefined}
                            target="_blank"
                            rel="noreferrer"
                          >
                            {text("mcp.presets.apply")}
                            <Link.ExternalIcon />
                          </Link>
                        </>
                      )}
                    </Table.Cell>
                    <Table.Cell>
                      {preset.enabled ? (
                        <Button type="button" disabled>
                          {text("mcp.presets.enabled")}
                        </Button>
                      ) : preset.requires_key ? (
                        <Button type="button" onClick={() => onConfigure(preset)}>
                          {text("mcp.presets.configure")}
                        </Button>
                      ) : (
                        <Button
                          type="button"
                          disabled={enablingId === preset.id}
                          onClick={() => onEnable(preset)}
                        >
                          {enablingId === preset.id
                            ? text("mcp.presets.enabling")
                            : text("mcp.presets.enable")}
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
