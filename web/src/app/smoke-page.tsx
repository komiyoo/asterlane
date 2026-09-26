import { Button, Dialog, Input } from "@cloudflare/kumo";
import { useState, type ChangeEvent } from "react";

export function SmokePage() {
  const [note, setNote] = useState("");

  function onNoteChange(event: ChangeEvent<HTMLInputElement>) {
    setNote(event.target.value);
  }

  return (
    <main className="shell">
      <header className="topbar">
        <h1>
          星径 <span>控制台</span>
        </h1>
        <p className="hint">开发入口。管理 API 尚未接入，旧控制台仍由网关提供。</p>
      </header>
      <section className="panel">
        <h2>冒烟</h2>
        <Input label="备注" value={note} onChange={onNoteChange} placeholder="仅保存在本页" />
        <p className="note-preview">{note === "" ? "未填写" : note}</p>
        <Dialog.Root>
          <Dialog.Trigger render={(props) => <Button {...props}>打开说明</Button>} />
          <Dialog className="smoke-dialog">
            <Dialog.Title>开发入口</Dialog.Title>
            <Dialog.Description>
              这个对话框用来确认样式、键盘关闭和焦点恢复。按 Esc 关闭。
            </Dialog.Description>
            <Dialog.Close render={(props) => <Button {...props}>关闭</Button>} />
          </Dialog>
        </Dialog.Root>
        <ul className="status-list">
          <li className="status ok">样式入口已加载</li>
          <li className="status warn">尚未调用 /admin</li>
          <li className="status err">未连接网关</li>
        </ul>
      </section>
    </main>
  );
}
