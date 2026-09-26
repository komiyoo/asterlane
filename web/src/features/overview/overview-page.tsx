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
import { formatAverage } from "../format.ts";

export function OverviewPage() {
  const health = usePageLoad((signal) => getHealth({ signal }), []);
  const stats = usePageLoad((signal) => getStats({ signal }), []);

  return (
    <section className="page">
      <h2>总览</h2>
      <div className="cards">
        <HealthCard load={health} />
        <StatsCards load={stats} />
      </div>
    </section>
  );
}

function HealthCard({ load }: { load: ReturnType<typeof usePageLoad<OutputHealthResponse>> }) {
  if (load.loading && load.data === undefined) {
    return <LoadingState label="正在读取健康状态…" />;
  }
  if (load.error !== null && load.data === undefined) {
    return <ErrorState error={load.error} onRetry={load.reload} />;
  }
  if (load.data === undefined) {
    return <EmptyState title="没有健康数据" />;
  }
  return (
    <>
      <article className="card">
        <div className="num">
          {load.data.status} · v{load.data.version}
        </div>
        <div className="lbl">网关状态</div>
      </article>
      {load.error === null ? null : <ErrorState error={load.error} onRetry={load.reload} />}
    </>
  );
}

function StatsCards({ load }: { load: ReturnType<typeof usePageLoad<OutputStatsResponse>> }) {
  if (load.loading && load.data === undefined) {
    return <LoadingState label="正在读取统计…" />;
  }
  if (load.error !== null && load.data === undefined) {
    return <ErrorState error={load.error} onRetry={load.reload} />;
  }
  if (load.data === undefined) {
    return <EmptyState title="没有统计数据" />;
  }
  const stats = load.data;
  return (
    <>
      <Stat label="请求总数" value={String(stats.total_requests)} />
      <Stat label="错误数" value={String(stats.total_errors)} />
      <Stat label="平均延迟" value={`${formatAverage(stats.avg_latency_ms)} ms`} />
      <Stat label="限流命中" value={String(stats.total_rate_limit_hits)} />
      <Stat label="活跃工具" value={String(stats.unique_tools)} />
      <Stat label="活跃 proxy key" value={String(stats.unique_proxy_keys)} />
      <Stat label="活跃资源" value={String(stats.unique_resources)} />
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
