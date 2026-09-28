import { useEffect, useState } from "react";
import { ArrowDown, Plugs, PlugsConnected } from "@phosphor-icons/react";
import { api, onBrowsers } from "../lib/api";
import { useStore } from "../lib/store";
import * as fmt from "../lib/format";

export function StatusBar() {
  const jobs = useStore((s) => s.jobs);
  const queueRunning = useStore((s) => s.queueRunning);
  const limit = useStore((s) => s.settings?.speedLimit ?? 0);
  const [browsers, setBrowsers] = useState<string[]>([]);

  useEffect(() => {
    api.browserStatus().then((s) => setBrowsers(s.connected.map((c) => c.browser)));
    const un = onBrowsers((c) => setBrowsers(c.map((x) => x.browser)));
    return () => {
      un.then((f) => f());
    };
  }, []);

  let active = 0;
  let speed = 0;
  let queued = 0;
  for (const j of Object.values(jobs)) {
    if (j.state === "downloading") {
      active++;
      speed += j.speed;
    } else if (j.state === "queued") queued++;
  }

  return (
    <div className="statusbar">
      <span className="item">
        <ArrowDown size={13} />
        {active ? `${active} downloading, ${fmt.speed(speed) || "0 B/s"}` : "Idle"}
      </span>
      {queued > 0 && <span className="item">{queued} queued{queueRunning ? "" : " (on hold)"}</span>}
      {limit > 0 && <span className="item">Limited to {fmt.speed(limit)}</span>}
      <span className="item" style={{ marginLeft: "auto" }} title="Browsers with the CrowDM extension connected right now">
        {browsers.length ? <PlugsConnected size={14} /> : <Plugs size={14} />}
        {browsers.length ? `Connected: ${[...new Set(browsers)].map(cap).join(", ")}` : "No browser connected"}
      </span>
    </div>
  );
}

function cap(s: string) {
  return s ? s[0].toUpperCase() + s.slice(1) : s;
}
