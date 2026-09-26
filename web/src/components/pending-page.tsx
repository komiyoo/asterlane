import { EmptyState } from "./empty-state.tsx";

export function PendingPage({ title }: { title: string }) {
  return (
    <section className="page">
      <h2>{title}</h2>
      <EmptyState
        title="尚未迁移"
        description="此页面尚未迁移。不会创建、删除、签发、探测或调试。"
      />
    </section>
  );
}
