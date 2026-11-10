/**
 * @harnix/plugins — the settings section for Agix plugins (the `plugins` extension's interface):
 * what is installed, each one switched on or off or removed, what it contributes to the interface,
 * and a .zip installed from this machine.
 */

import { useRef, useState } from "react";
import { errorText } from "@harnix/core";
import { SettingsRow, SettingsSection, SettingsValue, t, useSettingsReload, useT, type HostApi, type Registry } from "@harnix/react";

/** The backend extension this is the interface of. */
export const extension = "plugins";

export function register(r: Registry, host: HostApi): void {
  r.register("settings.section", { id: "plugins", label: () => t("settings.nav.plugins"), render: () => <Plugins host={host} /> }, extension);
}

interface PluginList {
  available?: boolean;
  reason?: string;
  plugins?: { name: string; version?: string; enabled?: boolean; description?: string; script_faults?: string[] }[];
  contributes?: { plugin: string; ui?: { point: string; module: string }[]; inert?: string[]; notes?: string[] }[];
}

/* Plugins are Agix's. `PLUGINS_DISABLED` means this build carries no plugin subsystem at all, which
 * is a different fact from "nothing is installed", and is said as one. */
function Plugins({ host }: { host: HostApi }) {
  const t = useT();
  const reload = useSettingsReload();
  const act = (method: string, params: unknown) =>
    host
      .call(method, params)
      .then(reload)
      .catch((e: unknown) => host.shell.notice(errorText(e), "error"));
  return (
    <SettingsSection title={t("settings.plugins")} load={() => host.call<PluginList>("plugins.list", {})}>
      {(data) => {
        if (data.available === false) return <div className="hint">{data.reason || t("settings.pluginsUnavailable")}</div>;
        const plugins = data.plugins ?? [];
        return (
          <>
            <Installer host={host} />
            {!plugins.length ? <div className="hint">{t("settings.pluginsNone")}</div> : null}
            {plugins.map((plugin) => {
              const contributes = (data.contributes ?? []).find((c) => c.plugin === plugin.name);
              return (
                <div key={plugin.name}>
                  <SettingsRow label={`${plugin.name} ${plugin.version ?? ""}`} hint={plugin.description ?? ""}>
                    <button
                      type="button"
                      className="btn sm outline"
                      onClick={() => void act(plugin.enabled ? "plugins.disable" : "plugins.enable", { name: plugin.name })}
                    >
                      {plugin.enabled ? t("settings.pluginDisable") : t("settings.pluginEnable")}
                    </button>
                    <button
                      type="button"
                      className="btn sm danger"
                      onClick={() => {
                        void host.dialogs.confirm(t("settings.pluginRemoveConfirm", { name: plugin.name }), { danger: true }).then((yes) => (yes ? act("plugins.uninstall", { name: plugin.name }) : undefined));
                      }}
                    >
                      {t("common.remove")}
                    </button>
                  </SettingsRow>
                  {(contributes?.ui ?? []).map((entry) => (
                    <SettingsRow key={entry.module} label={t("settings.pluginContributes")} hint={entry.module}>
                      <SettingsValue>{entry.point}</SettingsValue>
                    </SettingsRow>
                  ))}
                  {(contributes?.inert ?? []).map((reason) => (
                    <SettingsRow key={reason} label={t("settings.pluginInert")} hint={reason} />
                  ))}
                  {(contributes?.notes ?? []).map((note) => (
                    <SettingsRow key={note} label={t("settings.pluginNote")} hint={note} />
                  ))}
                  {(plugin.script_faults ?? []).map((fault) => (
                    <SettingsRow key={fault} label={t("settings.pluginFault")} hint={fault} />
                  ))}
                </div>
              );
            })}
          </>
        );
      }}
    </SettingsSection>
  );
}

/* Agix fetches packages itself and has no upload channel; this sends the .zip up the bulk channel
 * and asks the engine to install it from where it landed. The file is not opened on this side. */
function Installer({ host }: { host: HostApi }) {
  const t = useT();
  const pick = useRef<HTMLInputElement>(null);
  const [status, setStatus] = useState<{ text: string; disclosure?: string } | null>(null);
  const install = async (file: File) => {
    setStatus({ text: t("settings.pluginUploading", { name: file.name }) });
    try {
      const grant = await host.call<{ url: string; path: string }>("plugins.upload", { name: file.name });
      const response = await fetch(grant.url, { method: "POST", body: file });
      if (!response.ok) throw new Error(await response.text());
      setStatus({ text: t("settings.pluginInstalling") });
      const result = await host.call<{ message?: string; disclosure?: unknown }>("plugins.install", { path: grant.path, name: file.name });
      setStatus({
        text: result.message || t("settings.pluginInstalled"),
        disclosure:
          result.disclosure == null ? undefined : typeof result.disclosure === "string" ? result.disclosure : JSON.stringify(result.disclosure, null, 2),
      });
    } catch (e) {
      setStatus({ text: errorText(e) });
    }
  };
  return (
    <div>
      <input
        ref={pick}
        type="file"
        accept=".zip"
        hidden
        onChange={(e) => {
          const file = e.target.files?.[0];
          e.target.value = "";
          if (file) void install(file);
        }}
      />
      <button type="button" className="btn sm outline" onClick={() => pick.current?.click()}>
        {t("settings.pluginInstall")}
      </button>
      <div className="hint">{t("settings.pluginInstallHint")}</div>
      {status ? (
        <div className="hint">
          {status.text}
          {status.disclosure ? <pre>{status.disclosure}</pre> : null}
        </div>
      ) : null}
    </div>
  );
}
