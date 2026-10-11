import {
  getHealth,
  getStats,
  type OutputHealthResponse,
  type OutputStatsResponse,
} from "../../api/index.ts";
import { usePageLoad } from "../../app/use-page-load.ts";
import { EmptyState } from "../../components/empty-state.tsx";
import { ErrorState } from "../../components/error-state.tsx";
import { LoadingState } from "../../components/loading-state.tsx";
import { text } from "../../i18n/locale.ts";
import { formatAverage, formatCount } from "../format.ts";

export function OverviewPage() {
  const health = usePageLoad((signal) => getHealth({ signal }), []);
  const stats = usePageLoad((signal) => getStats({ signal }), []);

  return (
    <section className="page">
      <h2>{text("shell.routes.overview")}</h2>
      <div className="cards">
        <HealthCard load={health} />
        <StatsCards load={stats} />
      </div>
    </section>
  );
}

function HealthCard({ load }: { load: ReturnType<typeof usePageLoad<OutputHealthResponse>> }) {
  if (load.loading && load.data === undefined) {
    return <LoadingState label={text("overview.loadingHealth")} />;
  }
  if (load.error !== null && load.data === undefined) {
    return <ErrorState error={load.error} onRetry={load.reload} />;
  }
  if (load.data === undefined) {
    return <EmptyState title={text("overview.emptyHealth")} />;
  }
  return (
    <>
      <article className="card">
        <div className="num">
          {load.data.status} · v{load.data.version}
        </div>
        <div className="lbl">{text("overview.gateway")}</div>
      </article>
      {load.error === null ? null : <ErrorState error={load.error} onRetry={load.reload} />}
    </>
  );
}

function StatsCards({ load }: { load: ReturnType<typeof usePageLoad<OutputStatsResponse>> }) {
  if (load.loading && load.data === undefined) {
    return <LoadingState label={text("overview.loadingStats")} />;
  }
  if (load.error !== null && load.data === undefined) {
    return <ErrorState error={load.error} onRetry={load.reload} />;
  }
  if (load.data === undefined) {
    return <EmptyState title={text("overview.emptyStats")} />;
  }
  const stats = load.data;
  return (
    <>
      <Stat label={text("overview.requests")} value={formatCount(stats.total_requests)} />
      <Stat label={text("overview.errors")} value={formatCount(stats.total_errors)} />
      <Stat label={text("overview.latency")} value={`${formatAverage(stats.avg_latency_ms)} ms`} />
      <Stat label={text("overview.rateLimit")} value={formatCount(stats.total_rate_limit_hits)} />
      <Stat label={text("overview.tools")} value={formatCount(stats.unique_tools)} />
      <Stat label={text("overview.proxyKeys")} value={formatCount(stats.unique_proxy_keys)} />
      <Stat label={text("overview.resources")} value={formatCount(stats.unique_resources)} />
      {load.error === null ? null : <ErrorState error={load.error} onRetry={load.reload} />}
    </>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <article className="card">
      <div className="num">{value}</div>
      <div className="lbl">{label}</div>
    </article>
  );
}
