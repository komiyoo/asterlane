export function LimitNote({ limit, count }: { limit: number; count: number }) {
  return (
    <p className="hint" role="note">
      本次查询 limit 为 {limit}，返回 {count} 条。服务端没有总页数，这不是全部历史。
    </p>
  );
}
