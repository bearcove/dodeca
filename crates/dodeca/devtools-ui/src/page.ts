import { connect, Driver, voxServiceMetadata } from "@bearcove/vox-core";
import { wsConnector } from "@bearcove/vox-ws";
import { DevtoolsServiceClient } from "./devtools.generated";
import {
  BrowserServiceDispatcher,
  type DevtoolsEvent,
  type ErrorInfo,
} from "./browser.generated";

type ConnectionState = "connecting" | "connected" | "disconnected";
type PanelTab = "errors" | "coverage" | "actions";

interface CoverageStatus {
  totalRules: number;
  implementedRules: number;
  verifiedRules: number;
  uncoveredRules: number;
  untestedRules: number;
  invalidReferences: number;
  staleReferences: number;
  implementationCoveragePercent: number;
  verificationCoveragePercent: number;
}

interface CoverageRuleSummary {
  id: string;
  implemented: boolean;
  verified: boolean;
  staleRefs: number;
}

interface CoverageSourceFileNav {
  file: string;
  totalReferences: number;
  implRefs: number;
  verifyRefs: number;
  invalidRefs: number;
  staleRefs: number;
  unmappedUnits: Array<{ file: string; line: number; endLine: number; kind: string; name: string | null }>;
}

interface CoverageNavigationResponse {
  specName: string;
  status: CoverageStatus;
  coverageRules: CoverageRuleSummary[];
  sourceFiles: CoverageSourceFileNav[];
}

interface PageDevtoolsState {
  connection: ConnectionState;
  route: string;
  open: boolean;
  tab: PanelTab;
  errors: Map<string, ErrorInfo>;
  coverage: CoverageNavigationResponse | null;
  coverageLoading: boolean;
  coverageError: string | null;
}

function wsUrl(): string {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  return `${proto}://${location.host}/_/ws`;
}

function normalizeRoute(route: string): string {
  return route.replace(/\/+$/, "") || "/";
}

function currentRoute(): string {
  return normalizeRoute(location.pathname);
}

function editHref(route: string): string {
  return `/_dodeca/edit/${route.replace(/^\/+/, "")}`;
}

function fmtPercent(value: number): string {
  return `${Math.round(value)}%`;
}

function loc(error: ErrorInfo): string {
  const parts = [];
  if (error.template) parts.push(error.template);
  if (error.line != null) parts.push(String(error.line));
  if (error.column != null) parts.push(String(error.column));
  return parts.join(":");
}

function clear(el: Element): void {
  while (el.firstChild) el.firstChild.remove();
}

function button(label: string, className: string, onClick: () => void): HTMLButtonElement {
  const el = document.createElement("button");
  el.type = "button";
  el.className = className;
  el.textContent = label;
  el.addEventListener("click", onClick);
  return el;
}

function metric(label: string, value: string, tone = ""): HTMLElement {
  const el = document.createElement("div");
  el.className = `ddt-metric ${tone}`;
  const strong = document.createElement("strong");
  strong.textContent = value;
  const span = document.createElement("span");
  span.textContent = label;
  el.append(strong, span);
  return el;
}

export function mountPageDevtools(): void {
  if (document.querySelector(".ddt-root")) return;

  const state: PageDevtoolsState = {
    connection: "connecting",
    route: currentRoute(),
    open: false,
    tab: "errors",
    errors: new Map(),
    coverage: null,
    coverageLoading: false,
    coverageError: null,
  };

  const root = document.createElement("div");
  root.className = "ddt-root";
  root.innerHTML = `
    <button class="ddt-badge" type="button" aria-expanded="false">
      <span class="ddt-dot"></span>
      <span class="ddt-badge-label">dodeca</span>
    </button>
    <section class="ddt-panel" hidden>
      <header class="ddt-head">
        <div>
          <div class="ddt-title">Dodeca</div>
          <div class="ddt-route"></div>
        </div>
        <button class="ddt-close" type="button" aria-label="Close">×</button>
      </header>
      <nav class="ddt-tabs" aria-label="DevTools tabs"></nav>
      <div class="ddt-body"></div>
    </section>
  `;
  document.body.appendChild(root);

  const badge = root.querySelector<HTMLButtonElement>(".ddt-badge")!;
  const badgeLabel = root.querySelector<HTMLElement>(".ddt-badge-label")!;
  const dot = root.querySelector<HTMLElement>(".ddt-dot")!;
  const panel = root.querySelector<HTMLElement>(".ddt-panel")!;
  const routeEl = root.querySelector<HTMLElement>(".ddt-route")!;
  const close = root.querySelector<HTMLButtonElement>(".ddt-close")!;
  const tabsEl = root.querySelector<HTMLElement>(".ddt-tabs")!;
  const body = root.querySelector<HTMLElement>(".ddt-body")!;

  const render = () => {
    const errorCount = state.errors.size;
    badge.setAttribute("aria-expanded", state.open ? "true" : "false");
    badgeLabel.textContent =
      state.connection === "connected" && errorCount > 0
        ? `${errorCount} error${errorCount === 1 ? "" : "s"}`
        : state.connection === "connected"
          ? "dodeca"
          : state.connection;
    dot.dataset.state = state.connection;
    panel.hidden = !state.open;
    routeEl.textContent = state.route;

    clear(tabsEl);
    const tabs: Array<[PanelTab, string]> = [
      ["errors", `Errors${errorCount > 0 ? ` ${errorCount}` : ""}`],
      ["coverage", "Coverage"],
      ["actions", "Actions"],
    ];
    for (const [id, label] of tabs) {
      const tab = button(label, `ddt-tab${state.tab === id ? " ddt-on" : ""}`, () => {
        state.tab = id;
        if (id === "coverage") void loadCoverage();
        render();
      });
      tabsEl.appendChild(tab);
    }

    clear(body);
    if (state.tab === "errors") renderErrors(body);
    else if (state.tab === "coverage") renderCoverage(body);
    else renderActions(body);
  };

  const renderErrors = (container: HTMLElement) => {
    if (state.errors.size === 0) {
      const empty = document.createElement("div");
      empty.className = "ddt-empty";
      empty.textContent = "No template errors";
      container.appendChild(empty);
      return;
    }

    for (const error of state.errors.values()) {
      const item = document.createElement("article");
      item.className = "ddt-error";
      const message = document.createElement("div");
      message.className = "ddt-error-message";
      message.textContent = error.message;
      item.appendChild(message);

      const locationText = loc(error);
      if (locationText) {
        const location = document.createElement("div");
        location.className = "ddt-muted";
        location.textContent = locationText;
        item.appendChild(location);
      }

      if (error.source_snippet) {
        const pre = document.createElement("pre");
        pre.className = "ddt-source";
        pre.textContent = error.source_snippet.lines
          .map((line) => `${String(line.number).padStart(4, " ")}  ${line.content}`)
          .join("\n");
        item.appendChild(pre);
      }
      container.appendChild(item);
    }
  };

  const renderCoverage = (container: HTMLElement) => {
    if (state.coverageLoading) {
      const loading = document.createElement("div");
      loading.className = "ddt-empty";
      loading.textContent = "Loading coverage";
      container.appendChild(loading);
      return;
    }

    if (state.coverageError || !state.coverage) {
      const empty = document.createElement("div");
      empty.className = "ddt-empty";
      empty.textContent = state.coverageError ?? "No coverage report";
      container.appendChild(empty);
      container.appendChild(actionLink("Open coverage navigation", "/_dodeca/coverage/"));
      return;
    }

    const status = state.coverage.status;
    const metrics = document.createElement("div");
    metrics.className = "ddt-metrics";
    metrics.append(
      metric("implemented", fmtPercent(status.implementationCoveragePercent)),
      metric("verified", fmtPercent(status.verificationCoveragePercent)),
      metric("uncovered", String(status.uncoveredRules), status.uncoveredRules > 0 ? "ddt-warn" : ""),
      metric("untested", String(status.untestedRules), status.untestedRules > 0 ? "ddt-warn" : ""),
      metric("invalid", String(status.invalidReferences), status.invalidReferences > 0 ? "ddt-bad" : ""),
      metric("stale", String(status.staleReferences), status.staleReferences > 0 ? "ddt-bad" : ""),
    );
    container.appendChild(metrics);

    const heading = document.createElement("h3");
    heading.textContent = state.coverage.specName;
    container.appendChild(heading);

    const rules = state.coverage.coverageRules
      .filter((rule) => !rule.implemented || !rule.verified || rule.staleRefs > 0)
      .slice(0, 12);
    if (rules.length > 0) {
      const list = document.createElement("div");
      list.className = "ddt-list";
      for (const rule of rules) {
        const row = document.createElement("a");
        row.className = "ddt-row";
        row.href = `/_dodeca/coverage/rule/${encodeURIComponent(rule.id)}.html`;
        row.textContent = rule.id;
        const statusText = document.createElement("span");
        statusText.textContent = [
          !rule.implemented ? "uncovered" : "",
          !rule.verified ? "untested" : "",
          rule.staleRefs > 0 ? `${rule.staleRefs} stale` : "",
        ]
          .filter(Boolean)
          .join(" · ");
        row.appendChild(statusText);
        list.appendChild(row);
      }
      container.appendChild(list);
    }

    container.appendChild(actionLink("Open coverage navigation", "/_dodeca/coverage/"));
  };

  const renderActions = (container: HTMLElement) => {
    const actions = document.createElement("div");
    actions.className = "ddt-actions";
    actions.append(
      actionLink("Edit this page", editHref(state.route)),
      actionLink("Coverage navigation", "/_dodeca/coverage/"),
      actionLink("Coverage markdown", "/_dodeca/coverage/nav.md"),
      actionLink("Coverage JSON", "/_dodeca/coverage/nav.json"),
    );
    container.appendChild(actions);
  };

  const actionLink = (label: string, href: string): HTMLAnchorElement => {
    const a = document.createElement("a");
    a.className = "ddt-action";
    a.href = href;
    a.textContent = label;
    return a;
  };

  const loadCoverage = async () => {
    if (state.coverage || state.coverageLoading) return;
    state.coverageLoading = true;
    state.coverageError = null;
    render();
    try {
      const response = await fetch("/_dodeca/coverage/nav.json", {
        headers: { Accept: "application/json" },
      });
      if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
      state.coverage = (await response.json()) as CoverageNavigationResponse;
    } catch (err) {
      state.coverageError = String(err);
    } finally {
      state.coverageLoading = false;
      render();
    }
  };

  const handleEvent = async (event: DevtoolsEvent): Promise<void> => {
    switch (event.tag) {
      case "Error":
        state.errors.set(normalizeRoute(event.value.route), event.value);
        state.tab = "errors";
        state.open = true;
        break;
      case "ErrorResolved":
        state.errors.delete(normalizeRoute(event.route));
        break;
      case "Reload":
      case "CssChanged":
      case "Patches":
        break;
    }
    render();
  };

  const connectDevtools = async () => {
    state.connection = "connecting";
    render();
    try {
      const connection = await connect(wsConnector(wsUrl()));
      const lane = await connection.openRawLane({
        metadata: voxServiceMetadata("DevtoolsService"),
      });
      void Driver.new(lane, new BrowserServiceDispatcher({ onEvent: handleEvent }))
        .run()
        .catch(() => {})
        .finally(() => {
          state.connection = "disconnected";
          render();
        });
      const client = new DevtoolsServiceClient(lane.caller());
      await client.subscribe(state.route);
      state.connection = "connected";
      render();
    } catch (err) {
      console.error("[dodeca-devtools] connect failed:", err);
      state.connection = "disconnected";
      render();
    }
  };

  badge.addEventListener("click", () => {
    state.open = !state.open;
    if (state.open && state.tab === "coverage") void loadCoverage();
    render();
  });
  close.addEventListener("click", () => {
    state.open = false;
    render();
  });

  window.setInterval(() => {
    const route = currentRoute();
    if (route !== state.route) {
      state.route = route;
      render();
    }
  }, 300);

  render();
  void connectDevtools();
}
