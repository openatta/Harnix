/**
 * The right column for an instance: its facts, its restart and stop, its sessions, and a new instance.
 * Stopping, restarting and building are the system module's (`system.instances.*`): it holds
 * `instances.admin`, and the page, a device, holds no capability.
 */

import { useEffect, useMemo, useState } from "react";
import { useT, Glyph } from "@harnix/react";

import { SYSTEM } from "../constants.ts";
import { statusReason, type InstanceRow, type SessionRowView } from "../types.ts";
import { act, useTopic, type View } from "../view.ts";
import { Facts } from "./facts.tsx";

interface Template {
  id: string;
  carrier: string;
  params: string[];
}

export function InstanceOverview({ view, instance: id }: { view: View; instance: string }) {
  const host = view.host;
  const tr = useT();
  const listed = useTopic<{ instances: InstanceRow[] }>(view, "harnix.instances");
  const instance = listed?.instances.find((i) => i.id === id);
  return (
    <div className="system-pane" data-testid="system-instance">
      {instance ? (
        <>
          <Facts
            rows={[
              [tr("system.fact.instance"), instance.id],
              [tr("system.fact.state"), statusReason(instance.status) ? `${instance.status.state} — ${statusReason(instance.status)}` : instance.status.state],
              [tr("system.fact.carrier"), tr(`system.carrier.${instance.carrier}`)],
              [tr("system.fact.template"), instance.template ?? ""],
              [tr("system.fact.owner"), instance.owner],
              [tr("system.fact.version"), instance.version ? `${instance.version} · v${instance.protocol_version ?? "?"}` : ""],
              [tr("system.fact.scenes"), (instance.scenes ?? []).join(", ")],
              [tr("system.fact.pid"), instance.pid != null ? String(instance.pid) : ""],
              [tr("system.fact.dataRoot"), instance.data_root ?? ""],
            ]}
          />
          {instance.id !== host.defaults.instance ? (
            <div className="ops">
              <button
                type="button"
                className="btn sm outline op"
                data-testid="inst-restart"
                onClick={() => void act(host, () => host.call("system.instances.restart", { instance: instance.id, space: SYSTEM }))}
              >
                <Glyph name="refresh" className="glyph" />
                {tr("system.restart")}
              </button>
              {instance.owner !== "kernel" ? (
                <button
                  type="button"
                  className="btn sm outline op danger"
                  data-testid="inst-stop"
                  onClick={() =>
                    void host.dialogs.confirm(tr("system.confirmStop", { instance: instance.id }), { danger: true }).then(
                      (yes) =>
                        yes &&
                        act(host, async () => {
                          await host.call("system.instances.stop", { instance: instance.id, space: SYSTEM });
                          view.select({ kind: "instance", instance: host.defaults.instance });
                        }),
                    )
                  }
                >
                  <Glyph name="stop" className="glyph" />
                  {tr("system.stop")}
                </button>
              ) : null}
            </div>
          ) : null}
          <div className="label">{tr("system.sessionsOf")}</div>
          <InstanceSessions view={view} instance={instance.id} />
        </>
      ) : (
        <div className="empty">{tr("system.pickInstance")}</div>
      )}
      <div className="label">{tr("system.newInstance")}</div>
      <NewInstance view={view} />
    </div>
  );
}

/** Every session of an instance, running or not, of whatever space: one click to inspect it. */
function InstanceSessions({ view, instance }: { view: View; instance: string }) {
  const tr = useT();
  const params = useMemo(() => ({ instance }), [instance]);
  const listed = useTopic<{ sessions: SessionRowView[] }>(view, "harnix.instances.sessions", params);
  const rows = [...(listed?.sessions ?? [])].sort((a, b) => String(b.last_active ?? "").localeCompare(String(a.last_active ?? "")));
  if (!rows.length) return <div className="note">{tr("sidebar.noSessions")}</div>;
  return (
    <div className="inst-sessions" data-instance={instance}>
      {rows.map((row) => (
        <button
          type="button"
          key={row.session_id}
          className="srow sub"
          data-testid="instance-session"
          data-sid={row.session_id}
          data-instance={instance}
          title={[row.name, row.scene_layer ?? row.scene, row.project_root].filter(Boolean).join(" · ")}
          onClick={() => view.select({ kind: "session", instance, sessionId: row.session_id })}
        >
          <span className="tt name">{row.name || tr("sidebar.untitled")}</span>
          <span className="space-tag" data-space={row.space}>
            {row.space ?? view.host.defaults.space ?? ""}
          </span>
        </button>
      ))}
    </div>
  );
}

function NewInstance({ view }: { view: View }) {
  const host = view.host;
  const tr = useT();
  const [templates, setTemplates] = useState<Template[] | null>(null);
  const [template, setTemplate] = useState("");
  const [name, setName] = useState("");
  const [values, setValues] = useState<Record<string, string>>({});
  useEffect(() => {
    void host
      .call<{ templates: Template[] }>("system.instances.templates", { space: SYSTEM })
      .then((r) => {
        setTemplates(r.templates ?? []);
        setTemplate((r.templates ?? [])[0]?.id ?? "");
      })
      .catch(() => setTemplates([]));
  }, [host]);
  if (!templates) return null;
  if (!templates.length) return <div className="note">{tr("system.noTemplates")}</div>;
  const chosen = templates.find((x) => x.id === template);
  const create = () =>
    void act(host, async () => {
      const built = await host.call<{ id?: string; instance?: { id: string } }>("system.instances.create", {
        template,
        name: name.trim(),
        params: Object.fromEntries((chosen?.params ?? []).map((p) => [p, values[p] ?? ""])),
        space: SYSTEM,
      });
      setName("");
      view.select({ kind: "instance", instance: built.instance?.id ?? built.id ?? name.trim() });
    });
  return (
    <div className="new-instance" data-testid="new-instance">
      <div className="field">
        <label>{tr("system.fact.template")}</label>
        <select value={template} onChange={(e) => setTemplate(e.target.value)}>
          {templates.map((x) => (
            <option key={x.id} value={x.id}>
              {x.id} · {tr(`system.carrier.${x.carrier}`)}
            </option>
          ))}
        </select>
      </div>
      <div className="field">
        <label>{tr("common.name")}</label>
        <input type="text" data-testid="new-instance-name" value={name} onChange={(e) => setName(e.target.value)} />
      </div>
      {(chosen?.params ?? []).map((param) => (
        <div className="field" key={param}>
          <label>{param}</label>
          <input type="text" value={values[param] ?? ""} onChange={(e) => setValues({ ...values, [param]: e.target.value })} />
        </div>
      ))}
      <button type="button" className="btn sm primary" data-testid="new-instance-create" disabled={!name.trim() || !template} onClick={create}>
        {tr("system.create")}
      </button>
    </div>
  );
}
