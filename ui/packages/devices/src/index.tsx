/**
 * @harnix/devices — the settings section for paired devices (the `devices` extension's interface):
 * who may connect, each one revoked, and a new one paired with a one-time code.
 */

import { useEffect, useState } from "react";
import { errorText } from "@harnix/core";
import { SettingsRow, SettingsSection, t, useSettingsReload, useT, type HostApi, type Registry } from "@harnix/react";

/** The backend extension this is the interface of. */
export const extension = "devices";

export function register(r: Registry, host: HostApi): void {
  r.register("settings.section", { id: "devices", label: () => t("settings.nav.devices"), render: () => <Devices host={host} /> }, extension);
}

/* The pairing code exists in the clear only here, once; it is read off and typed into the other
 * machine, hence large, monospaced and counting down. */
function Devices({ host }: { host: HostApi }) {
  const t = useT();
  const reload = useSettingsReload();
  const [grant, setGrant] = useState<{ code: string; left: number } | { error: string } | null>(null);
  useEffect(() => {
    if (!grant || !("code" in grant) || grant.left <= 0) return;
    const tick = setTimeout(() => setGrant({ ...grant, left: grant.left - 1 }), 1000);
    return () => clearTimeout(tick);
  }, [grant]);
  return (
    <SettingsSection title={t("settings.devices")} load={() => host.call<{ devices?: { id: string; label?: string }[] }>("devices.list", {})}>
      {(data) => (
        <>
          <div className="hint">{t("settings.devicesHint")}</div>
          {!(data.devices ?? []).length ? <div className="hint">{t("settings.devicesNone")}</div> : null}
          {(data.devices ?? []).map((device) => (
            <SettingsRow key={device.id} label={device.label || device.id} hint={device.id}>
              <button
                type="button"
                className="btn sm danger"
                data-testid="device-revoke"
                data-device={device.id}
                onClick={() => {
                  void host.dialogs
                    .confirm(t("settings.deviceRevokeConfirm", { name: device.label || device.id }), { danger: true })
                    .then((yes) => (yes ? host.call("devices.revoke", { device_id: device.id }).then(reload) : undefined))
                    .catch((e: unknown) => host.shell.notice(errorText(e), "error"));
                }}
              >
                {t("settings.deviceRevoke")}
              </button>
            </SettingsRow>
          ))}
          <button
            type="button"
            className="btn sm outline"
            onClick={() =>
              host
                .call<{ code: string; expires_in_secs: number }>("devices.pair.begin", { label: t("settings.deviceNew") })
                .then((g) => setGrant({ code: g.code, left: g.expires_in_secs }))
                .catch((e: unknown) => setGrant({ error: errorText(e) }))
            }
          >
            {t("settings.devicePair")}
          </button>
          {grant && "error" in grant ? <div className="hint">{grant.error}</div> : null}
          {grant && "code" in grant ? (
            grant.left > 0 ? (
              <div>
                <div className="pairing-code">{grant.code}</div>
                <div className="hint">{t("settings.devicePairHint")}</div>
                <span className="hint">
                  {t("settings.devicePairLeft", { minutes: Math.floor(grant.left / 60), seconds: String(grant.left % 60).padStart(2, "0") })}
                </span>
              </div>
            ) : (
              <div className="hint">{t("settings.devicePairExpired")}</div>
            )
          ) : null}
        </>
      )}
    </SettingsSection>
  );
}
