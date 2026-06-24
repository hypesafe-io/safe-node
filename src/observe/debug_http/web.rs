pub(super) const INDEX_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>safe-node</title>
  <style>
    :root {
      color-scheme: light;
      --bg: #f7f8fa;
      --panel: #ffffff;
      --line: #d7dde5;
      --text: #17202a;
      --muted: #5d6978;
      --accent: #176b5f;
      --accent-strong: #0f5147;
      --warn: #9d4d14;
      --bad: #9f2d2d;
      --good-bg: #e7f4ef;
      --warn-bg: #fff2df;
      --bad-bg: #fbe7e7;
    }

    * {
      box-sizing: border-box;
    }

    body {
      margin: 0;
      background: var(--bg);
      color: var(--text);
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont,
        "Segoe UI", sans-serif;
      font-size: 14px;
      line-height: 1.5;
    }

    main {
      width: min(1180px, calc(100% - 32px));
      margin: 0 auto;
      padding: 24px 0 32px;
    }

    header {
      display: flex;
      align-items: flex-end;
      justify-content: space-between;
      gap: 16px;
      margin-bottom: 20px;
    }

    h1,
    h2,
    h3 {
      margin: 0;
      letter-spacing: 0;
    }

    h1 {
      font-size: 26px;
      line-height: 1.15;
      font-weight: 700;
    }

    h2 {
      font-size: 15px;
      line-height: 1.3;
      font-weight: 700;
    }

    h3 {
      font-size: 13px;
      line-height: 1.25;
      font-weight: 700;
    }

    .subtle {
      color: var(--muted);
      font-size: 13px;
    }

    .status-line {
      display: flex;
      align-items: center;
      justify-content: flex-end;
      gap: 10px;
      min-height: 26px;
      color: var(--muted);
      white-space: nowrap;
    }

    .dot {
      width: 10px;
      height: 10px;
      border-radius: 50%;
      background: var(--warn);
      box-shadow: 0 0 0 3px var(--warn-bg);
    }

    .dot.ok {
      background: var(--accent);
      box-shadow: 0 0 0 3px var(--good-bg);
    }

    .dot.bad {
      background: var(--bad);
      box-shadow: 0 0 0 3px var(--bad-bg);
    }

    .grid {
      display: grid;
      grid-template-columns: repeat(12, 1fr);
      gap: 14px;
      align-items: start;
    }

    section {
      background: var(--panel);
      border: 1px solid var(--line);
      border-radius: 8px;
      min-width: 0;
    }

    .span-4 {
      grid-column: span 4;
    }

    .span-12 {
      grid-column: span 12;
    }

    .section-head {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      padding: 14px 16px 10px;
      border-bottom: 1px solid var(--line);
    }

    dl {
      display: grid;
      grid-template-columns: minmax(108px, 0.32fr) minmax(0, 1fr);
      gap: 8px 12px;
      margin: 0;
      padding: 14px 16px 16px;
    }

    dt {
      color: var(--muted);
    }

    dd {
      margin: 0;
      min-width: 0;
      overflow-wrap: anywhere;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace;
      font-size: 13px;
    }

    table {
      width: 100%;
      border-collapse: collapse;
      table-layout: fixed;
    }

    th,
    td {
      padding: 10px 12px;
      border-bottom: 1px solid var(--line);
      text-align: left;
      vertical-align: top;
      overflow-wrap: anywhere;
    }

    th {
      color: var(--muted);
      font-size: 12px;
      font-weight: 700;
      text-transform: uppercase;
    }

    tr:last-child td {
      border-bottom: 0;
    }

    .mono {
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace;
      font-size: 13px;
    }

    .pill {
      display: inline-flex;
      align-items: center;
      min-height: 22px;
      padding: 2px 8px;
      border-radius: 999px;
      background: #eef1f5;
      color: var(--text);
      font-size: 12px;
      font-weight: 700;
      white-space: nowrap;
    }

    .pill.ok {
      background: var(--good-bg);
      color: var(--accent-strong);
    }

    .pill.warn {
      background: var(--warn-bg);
      color: var(--warn);
    }

    .pill.bad {
      background: var(--bad-bg);
      color: var(--bad);
    }

    .empty {
      padding: 22px 16px;
      color: var(--muted);
    }

    .templates {
      display: flex;
      flex-wrap: wrap;
      gap: 6px;
    }

    .table-wrap {
      overflow-x: auto;
    }

    .risk-summary {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      padding: 14px 16px;
      border-bottom: 1px solid var(--line);
      background: #fbfcfd;
    }

    .risk-summary strong {
      display: block;
      font-size: 14px;
    }

    .risk-summary span {
      color: var(--muted);
      font-size: 13px;
    }

    .risk-badges {
      display: flex;
      flex-wrap: wrap;
      justify-content: flex-end;
      gap: 6px;
    }

    .risk-table,
    .transactions-table {
      min-width: 760px;
    }

    .risk-table th:nth-child(1) {
      width: 18%;
    }

    .risk-table th:nth-child(2) {
      width: 30%;
    }

    .risk-table th:nth-child(3) {
      width: 30%;
    }

    .risk-table th:nth-child(4) {
      width: 22%;
    }

    .risk-value {
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace;
      font-size: 13px;
    }

    .risk-text {
      color: var(--muted);
    }

    .risk-decision {
      color: var(--accent-strong);
      font-weight: 700;
    }

    @media (max-width: 980px) {
      .span-4,
      .span-12 {
        grid-column: span 12;
      }
    }

    @media (max-width: 640px) {
      main {
        width: min(100% - 20px, 680px);
        padding-top: 16px;
      }

      header {
        align-items: flex-start;
        flex-direction: column;
      }

      .status-line {
        justify-content: flex-start;
        white-space: normal;
      }

      dl {
        grid-template-columns: 1fr;
      }

      .risk-summary {
        align-items: flex-start;
        flex-direction: column;
      }

      .risk-badges {
        justify-content: flex-start;
      }

      .transactions-table th:nth-child(3),
      .transactions-table td:nth-child(3),
      .transactions-table th:nth-child(5),
      .transactions-table td:nth-child(5) {
        display: none;
      }
    }
  </style>
</head>
<body>
  <main>
    <header>
      <div>
        <h1>safe-node</h1>
        <div class="subtle">Local debug view</div>
      </div>
      <div class="status-line">
        <span id="health-dot" class="dot"></span>
        <span id="health-text">Loading</span>
      </div>
    </header>

    <div class="grid">
      <section class="span-4">
        <div class="section-head">
          <h2>Runtime</h2>
          <span id="mode" class="pill">unknown</span>
        </div>
        <dl id="runtime"></dl>
      </section>

      <section class="span-4">
        <div class="section-head">
          <h2>Authorization Scope</h2>
          <span class="pill ok">configured</span>
        </div>
        <dl id="scope"></dl>
      </section>

      <section class="span-4">
        <div class="section-head">
          <h2>System</h2>
          <span id="dry-run" class="pill">dry_run</span>
        </div>
        <dl id="system"></dl>
      </section>

      <section class="span-12">
        <div class="section-head">
          <h2>Risk Controls</h2>
          <span class="pill">read only</span>
        </div>
        <div id="risk-controls"></div>
      </section>

      <section class="span-12">
        <div class="section-head">
          <h2>Recent Transactions</h2>
          <span id="transaction-count" class="subtle">0 rows</span>
        </div>
        <div id="transactions"></div>
      </section>
    </div>
  </main>

  <script>
    const refreshMs = 2000;
    const endpoint = {
      status: "/debug/status",
      config: "/debug/config",
      policy: "/debug/policy",
      transactions: "/debug/transactions?limit=25",
    };

    function text(value) {
      if (value === null || value === undefined || value === "") {
        return "-";
      }
      return String(value);
    }

    function time(value) {
      if (!value) {
        return "-";
      }
      return new Date(value * 1000).toLocaleString();
    }

    function row(label, value) {
      return `<dt>${label}</dt><dd>${value}</dd>`;
    }

    function escapeHtml(value) {
      return text(value).replace(/[&<>"']/g, (char) => ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&#39;",
      })[char]);
    }

    async function fetchJson(url) {
      const response = await fetch(url, { cache: "no-store" });
      if (!response.ok) {
        throw new Error(`${response.status} ${response.statusText}`);
      }
      return response.json();
    }

    function renderRuntime(status) {
      document.getElementById("mode").textContent = text(status.mode);
      document.getElementById("runtime").innerHTML = [
        row("signer", escapeHtml(status.signer)),
        row("last poll", escapeHtml(time(status.last_poll_at))),
        row("last success", escapeHtml(time(status.last_success_at))),
        row("last error", escapeHtml(status.last_error)),
        row("last error at", escapeHtml(time(status.last_error_at))),
        row("gateway failures", escapeHtml(status.consecutive_gateway_failures)),
      ].join("");
    }

    function renderScope(policy) {
      document.getElementById("scope").innerHTML = [
        row("multisig", escapeHtml(policy.multisig)),
        row("leader", escapeHtml(policy.leader)),
      ].join("");
    }

    function renderSystem(config) {
      const dryRun = document.getElementById("dry-run");
      dryRun.textContent = config.dry_run ? "dry run" : "live";
      dryRun.className = config.dry_run ? "pill warn" : "pill ok";
      document.getElementById("system").innerHTML = [
        row("gateway", escapeHtml(config.gateway_url)),
        row("hl api", escapeHtml(config.hl_api_url)),
        row("poll interval", `${escapeHtml(config.poll_interval_secs)}s`),
        row("database", escapeHtml(config.state_db)),
        row("rpc addr", escapeHtml(config.rpc_http_addr)),
      ].join("");
    }

    function renderRiskControls(policy, config) {
      const templates = (policy.allowed_templates || [])
        .map((item) => `<span class="pill ok">${escapeHtml(item)}</span>`)
        .join("");
      const modeClass = config.dry_run ? "pill warn" : "pill ok";
      const rejectAction = config.dry_run
        ? "Record local reject only"
        : "Submit gateway reject vote";

      document.getElementById("risk-controls").innerHTML = `
        <div class="risk-summary">
          <div>
            <strong>Default deny: a task is allowed only when every rule passes.</strong>
            <span>Rejected tasks do not request signing payloads or submit Hyperliquid actions.</span>
          </div>
          <div class="risk-badges">
            <span class="pill bad">default reject</span>
            <span class="${modeClass}">${escapeHtml(rejectAction)}</span>
          </div>
        </div>
        <div class="table-wrap">
          <table class="risk-table">
            <thead>
              <tr>
                <th>Rule</th>
                <th>Current value</th>
                <th>Reject when</th>
                <th>Effect</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td><strong>Multisig</strong></td>
                <td class="risk-value">${escapeHtml(policy.multisig)}</td>
                <td class="risk-text">task multisig does not match this address</td>
                <td class="risk-decision">reject before signing</td>
              </tr>
              <tr>
                <td><strong>Leader</strong></td>
                <td class="risk-value">${escapeHtml(policy.leader)}</td>
                <td class="risk-text">task leader does not match this address</td>
                <td class="risk-decision">reject before signing</td>
              </tr>
              <tr>
                <td><strong>Template</strong></td>
                <td><div class="templates">${templates || "-"}</div></td>
                <td class="risk-text">template id is absent from allowed_templates</td>
                <td class="risk-decision">reject before payload</td>
              </tr>
              <tr>
                <td><strong>Amount</strong></td>
                <td class="risk-value">${escapeHtml(policy.withdraw_limit)}</td>
                <td class="risk-text">inputs.amount is missing, invalid, or greater than the limit</td>
                <td class="risk-decision">reject before payload</td>
              </tr>
              <tr>
                <td><strong>Reject action</strong></td>
                <td><span class="${modeClass}">${escapeHtml(rejectAction)}</span></td>
                <td class="risk-text">any policy rule fails</td>
                <td class="risk-decision">no signature, no HL submit</td>
              </tr>
            </tbody>
          </table>
        </div>
      `;
    }

    function renderTransactions(items) {
      document.getElementById("transaction-count").textContent = `${items.length} rows`;
      if (items.length === 0) {
        document.getElementById("transactions").innerHTML =
          '<div class="empty">No local transactions</div>';
        return;
      }

      const rows = items.map((item) => {
        const statusClass = item.local_status === "failed" || item.local_status === "reject"
          ? "bad"
          : "ok";
        return `
          <tr>
            <td class="mono">${escapeHtml(item.task_id)}</td>
            <td>${escapeHtml(item.template_id)} v${escapeHtml(item.template_version)}</td>
            <td class="mono">${escapeHtml(item.nonce)}</td>
            <td><span class="pill ${statusClass}">${escapeHtml(item.local_status)}</span></td>
            <td>${escapeHtml(time(item.updated_at))}</td>
            <td>${escapeHtml(item.reject_reason)}</td>
          </tr>
        `;
      }).join("");

      document.getElementById("transactions").innerHTML = `
        <div class="table-wrap">
        <table class="transactions-table">
          <thead>
            <tr>
              <th style="width: 28%">Task</th>
              <th style="width: 19%">Template</th>
              <th style="width: 9%">Nonce</th>
              <th style="width: 14%">Status</th>
              <th style="width: 18%">Updated</th>
              <th style="width: 12%">Reason</th>
            </tr>
          </thead>
          <tbody>${rows}</tbody>
        </table>
        </div>
      `;
    }

    function setHealth(ok, detail) {
      const dot = document.getElementById("health-dot");
      dot.className = ok ? "dot ok" : "dot bad";
      document.getElementById("health-text").textContent = detail;
    }

    async function refresh() {
      try {
        const [status, config, policy, transactions] = await Promise.all([
          fetchJson(endpoint.status),
          fetchJson(endpoint.config),
          fetchJson(endpoint.policy),
          fetchJson(endpoint.transactions),
        ]);
        renderRuntime(status);
        renderScope(policy);
        renderSystem(config);
        renderRiskControls(policy, config);
        renderTransactions(transactions);
        setHealth(true, `Updated ${new Date().toLocaleTimeString()}`);
      } catch (error) {
        setHealth(false, error.message);
      }
    }

    refresh();
    setInterval(refresh, refreshMs);
  </script>
</body>
</html>
"#;
