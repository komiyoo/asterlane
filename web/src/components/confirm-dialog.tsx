import { Button, Dialog } from "@cloudflare/kumo";

export function ConfirmDialog({
  label,
  title,
  description,
  resource,
  confirmLabel,
  onConfirm,
}: {
  label: string;
  title: string;
  description: string;
  resource?: string;
  confirmLabel: string;
  onConfirm: () => void;
}) {
  return (
    <Dialog.Root role="alertdialog">
      <Dialog.Trigger render={(props) => <Button {...props}>{label}</Button>} />
      <Dialog className="confirm-dialog" size="sm">
        <Dialog.Title>{title}</Dialog.Title>
        <Dialog.Description>{description}</Dialog.Description>
        {resource === undefined ? null : <p>对象：{resource}</p>}
        <div className="confirm-actions">
          <Dialog.Close render={(props) => <Button {...props}>取消</Button>} />
          <Button type="button" variant="destructive" onClick={onConfirm}>
            {confirmLabel}
          </Button>
        </div>
      </Dialog>
    </Dialog.Root>
  );
}
