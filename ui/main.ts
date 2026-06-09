import { invoke } from "@tauri-apps/api/core";
import defaultServers from "../dns_list.txt?raw";
import "./style.css";

type CheckResult = {
  line: number;
  server: string;
  status: string;
  duration_ms?: number;
  answers?: string[];
  error?: string;
};

type BatchCheckResponse = {
  results: CheckResult[];
  total: number;
  ok: number;
  failed: number;
};

type ExpandServersResponse = {
  servers: string;
  changed: boolean;
  error?: string;
};

const form = document.getElementById("form") as HTMLFormElement;
const servers = document.getElementById("servers") as HTMLInputElement;
const dnsRows = document.getElementById("dnsRows") as HTMLTableSectionElement;
const checkButton = document.getElementById("checkButton") as HTMLButtonElement;
const successListWrap = document.getElementById("successListWrap") as HTMLLabelElement;
const successList = document.getElementById("successList") as HTMLTextAreaElement;
const successCount = document.getElementById("successCount") as HTMLSpanElement;

servers.value = defaultServers;

function serverLines() {
  return servers.value.split("\n");
}

function isCheckableLine(line: string) {
  const trimmed = line.trim();
  return trimmed !== "" && !trimmed.startsWith("#");
}

function syncServersFromRows() {
  const lines = Array.from(dnsRows.querySelectorAll<HTMLElement>(".server-cell")).map((cell) =>
    cell.textContent?.replace(/\n/g, " ") ?? "",
  );
  servers.value = lines.join("\n");
}

function addRow(line: string, index: number, resultText: string) {
  const tr = document.createElement("tr");
  tr.dataset.line = String(index + 1);

  const serverTd = document.createElement("td");
  serverTd.className = "server-col";
  const serverCell = document.createElement("div");
  serverCell.className = "server-cell";
  serverCell.contentEditable = "plaintext-only";
  serverCell.spellcheck = false;
  serverCell.textContent = line;
  serverCell.addEventListener("input", syncServersFromRows);
  serverCell.addEventListener("keydown", (event) => {
    if (event.key !== "Enter") return;
    event.preventDefault();
    syncServersFromRows();
    const lines = serverLines();
    lines.splice(index + 1, 0, "");
    servers.value = lines.join("\n");
    renderPending("");
    dnsRows.querySelector<HTMLElement>(`[data-line="${index + 2}"] .server-cell`)?.focus();
  });
  serverTd.appendChild(serverCell);

  const lineTd = document.createElement("td");
  lineTd.className = "line-col";
  lineTd.textContent = String(index + 1);

  const timeTd = document.createElement("td");
  timeTd.className = "time-col";
  const timeCell = document.createElement("div");
  timeCell.className = "time-cell result-pending";
  timeCell.textContent = isCheckableLine(line) ? resultText : "";
  timeTd.appendChild(timeCell);

  const resultTd = document.createElement("td");
  resultTd.className = "result-col";
  const resultCell = document.createElement("div");
  resultCell.className = "result-cell result-pending";
  resultTd.appendChild(resultCell);

  tr.append(serverTd, lineTd, timeTd, resultTd);
  dnsRows.appendChild(tr);
}

function renderPending(text: string) {
  const activeLine = Number(document.activeElement?.closest("tr")?.getAttribute("data-line") || 0);
  dnsRows.innerHTML = "";
  serverLines().forEach((line, index) => addRow(line, index, text));
  if (activeLine > 0) {
    dnsRows.querySelector<HTMLElement>(`[data-line="${activeLine}"] .server-cell`)?.focus();
  }
}

function renderGlobalError(message: string) {
  renderPending("");
  dnsRows.querySelectorAll("tr").forEach((tr) => {
    const line = serverLines()[Number(tr.getAttribute("data-line") || 1) - 1] || "";
    if (!isCheckableLine(line)) return;
    const result = tr.querySelector<HTMLElement>(".result-cell");
    const time = tr.querySelector<HTMLElement>(".time-cell");
    if (time) {
      time.textContent = "";
      time.className = "time-cell result-error";
      time.textContent = message;
    }
    if (result) {
      result.textContent = "";
      result.title = message;
      result.className = "result-cell result-error";
    }
  });
}

function errorLabel(error?: string) {
  const value = (error || "").toLowerCase();
  return value.includes("timeout") || value.includes("timed out") || value.includes("deadline") ? "timeout" : "error";
}

function renderResults(items: CheckResult[], total: number) {
  renderPending("");
  const successful: string[] = [];
  items.forEach((item) => {
    const row = dnsRows.querySelector<HTMLElement>(`[data-line="${item.line}"] .result-cell`);
    const time = dnsRows.querySelector<HTMLElement>(`[data-line="${item.line}"] .time-cell`);
    if (!row) return;
    const answers = item.answers?.length ? item.answers.join(", ") : "<empty>";
    row.className = `result-cell ${item.status === "ok" ? "result-ok" : "result-error"}`;
    if (time) {
      time.className = `time-cell ${item.status === "ok" ? "result-ok" : "result-error"}`;
      time.textContent = item.status === "ok" ? `${(item.duration_ms || 0).toFixed(2)}` : errorLabel(item.error);
    }
    row.title = item.error || answers;
    if (item.status === "ok") {
      row.textContent = answers;
      const serverCell = dnsRows.querySelector<HTMLElement>(`[data-line="${item.line}"] .server-cell`);
      successful.push(`${serverCell?.textContent?.trim() || item.server}    # line ${item.line}`);
    } else {
      row.textContent = "";
    }
  });
  successList.value = successful.join("\n");
  successCount.textContent = `${successful.length}/${total}`;
  successListWrap.style.display = successful.length ? "grid" : "none";
}

function getFormValues() {
  syncServersFromRows();
  const data = new FormData(form);
  return {
    servers: String(data.get("servers") || ""),
    domain: String(data.get("domain") || "google.com"),
    typeName: String(data.get("type") || "A"),
    expected: String(data.get("expected") || ""),
    bootstrap: String(data.get("bootstrap") || ""),
    timeout: String(data.get("timeout") || "5s"),
  };
}

renderPending("");

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  const values = getFormValues();
  checkButton.disabled = true;
  checkButton.textContent = "检测中...";
  successListWrap.style.display = "none";
  successList.value = "";
  successCount.textContent = "";
  try {
    const expanded = await invoke<ExpandServersResponse>("expand_servers", values);
    if (expanded.servers && expanded.servers !== servers.value) {
      servers.value = expanded.servers;
    }
  } catch {}
  try {
    renderPending("检测中...");
    const result = await invoke<BatchCheckResponse>("check_servers", getFormValues());
    checkButton.textContent = "检测";
    renderResults(result.results, result.total);
  } catch (error) {
    checkButton.textContent = "检测";
    renderGlobalError(errorLabel(String(error)));
  } finally {
    checkButton.disabled = false;
  }
});
