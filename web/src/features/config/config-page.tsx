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
import { text } from "../../i18n/locale.ts";
import { formatCount } from "../format.ts";
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
      setExportMessage(text("config.exported"));
    } catch (caught) {
      setExportError(true);
      setExportMessage(text("config.exportFailed", { detail: formatApiError(caught) }));
    }
  }

  return (
    <section className="page">
      <h2>{text("shell.routes.config")}</h2>
      <p className="hint">{text("config.hint")}</p>
      <div className="toolbar">
        <Button type="button" onClick={() => void runValidate()} disabled={loading}>
          {text("config.validate")}
        </Button>
        <Button type="button" onClick={() => void onExport()}>
          {text("config.export")}
        </Button>
      </div>
      {exportMessage === null ? null : (
        <FormMessage tone={exportError ? "error" : "info"}>{exportMessage}</FormMessage>
      )}
      {loading ? <LoadingState label={text("config.checking")} /> : null}
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
      <p className={report.valid ? "ok" : "err"}>
        {report.valid ? text("config.valid") : text("config.invalid")}
      </p>
      <p>
        {text("config.counts", {
          resources: formatCount(report.resource_count),
          keys: formatCount(report.proxy_key_count),
          mcp: formatCount(report.mcp_server_count),
        })}
      </p>
      <IssueList title={text("config.errors")} issues={errors} />
      <IssueList title={text("config.warnings")} issues={warnings} />
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
      <h3 className="section-title">
        {text("config.issueCount", { title, count: formatCount(issues.length) })}
      </h3>
      <ul className="issues">
        {issues.map((issue, index) => (
          <li key={`${issue.level}-${issue.target}-${index}`}>
            [{issue.level}] {issue.target}: {issue.message}
          </li>
        ))}
      </ul>
    </section>
  );
}
