import { Button } from "@cloudflare/kumo";
import { useEffect, useState } from "react";
import {
  exportConfigYaml,
  formatApiError,
  validateConfig,
  type OutputConfigValidateResponse,
} from "../../api/index.ts";
import { useRefresh } from "../../app/use-refresh.ts";
import { ErrorState } from "../../components/error-state.tsx";
import { FormMessage } from "../../components/form-message.tsx";
import { LoadingState } from "../../components/loading-state.tsx";
import { downloadYaml } from "./download.ts";

export function ConfigPage() {
  const { nonce } = useRefresh();
  const [report, setReport] = useState<OutputConfigValidateResponse | undefined>(undefined);
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(false);
  const [exportMessage, setExportMessage] = useState<string | null>(null);
  const [exportError, setExportError] = useState(false);
  const [checked, setChecked] = useState(false);

  async function runValidate() {
    setLoading(true);
    setError(null);
    setChecked(true);
    try {
      setReport(await validateConfig());
    } catch (caught) {
      setError(caught);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    if (!checked) {
      return;
    }
    void runValidate();
    // 只在用户已经校验过之后，跟随页头刷新重读。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nonce]);

  async function onExport() {
    setExportMessage(null);
    setExportError(false);
    try {
      const yaml = await exportConfigYaml();
      downloadYaml(yaml);
      setExportMessage("已导出");
    } catch (caught) {
      setExportError(true);
      setExportMessage(`导出失败：${formatApiError(caught)}`);
    }
  }

  return (
    <section className="page">
      <h2>配置</h2>
      <p className="hint">校验和导出只读取当前合并快照，不会修改配置。</p>
      <div className="toolbar">
        <Button type="button" onClick={() => void runValidate()} disabled={loading}>
          校验当前配置
        </Button>
        <Button type="button" onClick={() => void onExport()}>
          导出 YAML
        </Button>
      </div>
      {exportMessage === null ? null : (
        <FormMessage tone={exportError ? "error" : "info"}>{exportMessage}</FormMessage>
      )}
      {loading ? <LoadingState label="正在校验…" /> : null}
      {error !== null ? <ErrorState error={error} onRetry={() => void runValidate()} /> : null}
      {report === undefined || error !== null ? null : <ValidationReport report={report} />}
    </section>
  );
}

function ValidationReport({ report }: { report: OutputConfigValidateResponse }) {
  const errors = report.issues.filter((issue) => issue.level === "error");
  const warnings = report.issues.filter((issue) => issue.level === "warn");
  return (
    <div className="card">
      <p className={report.valid ? "ok" : "err"}>{report.valid ? "配置有效" : "配置存在问题"}</p>
      <p>
        资源 {report.resource_count} · Proxy Key {report.proxy_key_count} · MCP{" "}
        {report.mcp_server_count}
      </p>
      <IssueList title="错误" issues={errors} />
      <IssueList title="警告" issues={warnings} />
    </div>
  );
}

function IssueList({
  title,
  issues,
}: {
  title: string;
  issues: OutputConfigValidateResponse["issues"];
}) {
  if (issues.length === 0) {
    return null;
  }
  return (
    <section>
      <h3>
        {title}（{issues.length}）
      </h3>
      <ul>
        {issues.map((issue, index) => (
          <li key={`${issue.level}-${issue.target}-${index}`}>
            [{issue.level}] {issue.target}: {issue.message}
          </li>
        ))}
      </ul>
    </section>
  );
}
