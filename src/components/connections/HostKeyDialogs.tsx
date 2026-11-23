// C09 (unknown SSH host) and C10 (host key changed).
import { TriangleAlert } from "lucide-react";
import { useState } from "react";
import type { HostKeyChanged, UnknownHostKey } from "../../lib/api";
import { Button, Checkbox, Dialog, linkButton } from "../dialog";
import { t, tx } from "../../i18n";

const copy = (text: string) => void navigator.clipboard?.writeText(text);

/** "ssh-ed25519" → "ED25519". */
const keyLabel = (type: string) => type.replace(/^ssh-/, "").replace(/^ecdsa-sha2-/, "ECDSA ").toUpperCase();

export function UnknownHostDialog({ host, hostKey, onTrust, onCancel, nested }: {
  host: string;
  hostKey: UnknownHostKey;
  onTrust(addToKnownHosts: boolean): void;
  onCancel(): void;
  nested?: boolean;
}) {
  const [remember, setRemember] = useState(true);
  return (
    // No onSubmit: Enter must not approve a key. Cancel takes the initial focus.
    <Dialog
      title={t("Unknown SSH host")}
      width={560}
      offset={80}
      nested={nested}
      onClose={onCancel}
      footer={
        <>
          <Button data-autofocus onClick={onCancel}>
            {t("Cancel")}
          </Button>
          <Button variant="primary" onClick={() => onTrust(remember)}>
            {t("Trust and connect")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-s4">
        <p className="leading-[1.5]">
          {tx("The SSH server {host} is not in {file}, so its identity cannot be confirmed automatically.", {
            host: <span key="host" className="font-mono text-[12.5px]">{host}</span>,
            file: <span key="file" className="font-mono text-[12.5px]">~/.ssh/known_hosts</span>,
          })}
        </p>
        <div className="border border-line bg-panel">
          <div className="flex items-center justify-between border-b border-line-2 px-s3 py-s2">
            <span className="label">{t("{type} key fingerprint", { type: keyLabel(hostKey.key_type) })}</span>
            <button type="button" className={`text-[12px] ${linkButton}`} onClick={() => copy(hostKey.fingerprint)}>
              {t("Copy")}
            </button>
          </div>
          <code className="selectable block px-s3 py-s3 font-mono text-[12.5px] break-all">{hostKey.fingerprint}</code>
        </div>
        <p className="text-[12px] text-text-3">{t("Compare this with the fingerprint from your server administrator. Trust it only if they match.")}</p>
        <Checkbox checked={remember} onChange={setRemember}>
          {t("Add this host to known_hosts")}
        </Checkbox>
      </div>
    </Dialog>
  );
}

/** A security warning with no way to proceed. */
export function HostKeyChangedDialog({ change, onClose, nested }: { change: HostKeyChanged; onClose(): void; nested?: boolean }) {
  const where = change.line === null ? t("approved in this app") : t("line {line}", { line: change.line });
  const details = `SSH host key of ${change.host} has changed.\nSaved (${where}): ${change.saved}\nReceived: ${change.received}`;
  return (
    <Dialog
      alert
      danger
      band="danger"
      nested={nested}
      title={
        <span className="flex items-center gap-s2">
          <TriangleAlert size={16} className="text-danger" />
          {t("SSH host key has changed")}
        </span>
      }
      headerExtra={<span className="border border-danger px-[4px] py-[2px] font-heading text-[10px] leading-none font-semibold tracking-[.08em] text-danger">{t("CONNECTION REFUSED")}</span>}
      width={600}
      offset={60}
      onClose={onClose}
      footer={
        <>
          <Button variant="ghost" onClick={() => copy(details)}>
            {t("Copy details")}
          </Button>
          <Button data-autofocus onClick={onClose}>
            {t("Close")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-s4 leading-[1.5]">
        <p>
          {tx("{host} presented a different key than the one recorded for it. Either the server was reinstalled or replaced, or someone is intercepting this connection. Nothing was sent to the server.", {
            host: <span key="host" className="font-mono text-[12.5px]">{change.host}</span>,
          })}
        </p>
        <div className="border border-line bg-panel">
          <div className="grid grid-cols-[124px_minmax(0,1fr)] border-b border-line-2 px-s3 py-s2">
            <span className="label self-center">{t("Saved")} · {where}</span>
            <code className="selectable font-mono text-[12px] break-all">{change.saved || "—"}</code>
          </div>
          <div className="grid grid-cols-[124px_minmax(0,1fr)] px-s3 py-s2">
            <span className="label self-center !text-danger">{t("Received")}</span>
            <code className="selectable font-mono text-[12px] break-all">{change.received}</code>
          </div>
        </div>
        <p className="text-[12.5px] text-text-2">
          {t("Confirm the new fingerprint with your server administrator.")}{" "}
          {change.line === null
            ? t("If it is correct, remove this connection and add it again.")
            : tx("If it is correct, remove line {line} from {file} and connect again.", { line: change.line, file: <span key="file" className="font-mono text-[12px]">known_hosts</span> })}
        </p>
      </div>
    </Dialog>
  );
}
