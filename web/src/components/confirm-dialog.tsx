import { Button, Dialog } from "@cloudflare/kumo";
import { text } from "../i18n/locale.ts";

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
        {resource === undefined ? null : <p>{text("common.object", { name: resource })}</p>}
        <div className="confirm-actions">
          <Dialog.Close render={(props) => <Button {...props}>{text("common.cancel")}</Button>} />
          <Button type="button" variant="destructive" onClick={onConfirm}>
            {confirmLabel}
          </Button>
        </div>
      </Dialog>
    </Dialog.Root>
  );
}
