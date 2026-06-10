import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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

type CheckProgressEvent = {
  runId: string;
  result: CheckResult;
  completed: number;
  total: number;
  ok: number;
  failed: number;
};

const form = document.getElementById("form") as HTMLFormElement;
const servers = document.getElementById("servers") as HTMLInputElement;
const dnsRows = document.getElementById("dnsRows") as HTMLTableSectionElement;
const checkButton = document.getElementById("checkButton") as HTMLButtonElement;
const successListWrap = document.getElementById("successListWrap") as HTMLLabelElement;
const successList = document.getElementById("successList") as HTMLTextAreaElement;
const successCount = document.getElementById("successCount") as HTMLSpanElement;

servers.value = defaultServers;

let activeRunId = "";
let activeTotal = 0;
const successfulServers = new Map<number, string>();

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

function resetRunState(total = 0) {
  activeTotal = total;
  successfulServers.clear();
  successListWrap.style.display = "none";
  successList.value = "";
  successCount.textContent = total ? `0/${total}` : "";
}

function renderGlobalError(message: string) {
  renderPending("");
  dnsRows.querySelectorAll("tr").forEach((tr) => {
    const line = serverLines()[Number(tr.getAttribute("data-line") || 1) - 1] || "";
    if (!isCheckableLine(line)) return;
    const result = tr.querySelector<HTMLElement>(".result-cell");
    const time = tr.querySelector<HTMLElement>(".time-cell");
    if (time) {
      time.className = "time-cell result-error";
      time.textContent = errorLabel(message);
      bindErrorCopy(time, message);
    }
    if (result) {
      result.className = "result-cell result-error";
      result.textContent = errorLabel(message);
      bindErrorCopy(result, message);
    }
  });
}

function errorLabel(error?: string) {
  const value = (error || "").toLowerCase();
  return value.includes("timeout") || value.includes("timed out") || value.includes("deadline") ? "timeout" : "error";
}

async function copyText(text: string) {
  if (!text) return false;
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const area = document.createElement("textarea");
    area.value = text;
    area.setAttribute("readonly", "true");
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    const copied = document.execCommand("copy");
    document.body.removeChild(area);
    return copied;
  }
}

function markCopied(cell: HTMLElement, copied: boolean) {
  const label = cell.dataset.label || cell.textContent || "";
  cell.textContent = copied ? "copied" : "copy failed";
  window.setTimeout(() => {
    cell.textContent = label;
  }, 1200);
}

function bindErrorCopy(cell: HTMLElement | null, error?: string) {
  if (!cell) return;
  cell.classList.remove("copyable-cell");
  cell.removeAttribute("title");
  cell.onclick = null;
  delete cell.dataset.label;
  if (!error) return;

  const label = errorLabel(error);
  cell.dataset.label = label;
  cell.classList.add("copyable-cell");
  cell.title = `${error}\n\n点击复制`;
  cell.onclick = async () => {
    const copied = await copyText(error);
    markCopied(cell, copied);
  };
}

function renderResults(items: CheckResult[], total: number) {
  renderPending("");
  resetRunState(total);
  items.forEach((item) => {
    applyProgress(item);
  });
  successCount.textContent = `${successfulServers.size}/${total}`;
}

function updateSuccessList() {
  const lines = Array.from(successfulServers.entries())
    .sort((a, b) => a[0] - b[0])
    .map(([, value]) => value);
  successList.value = lines.join("\n");
  successCount.textContent = `${successfulServers.size}/${activeTotal}`;
  successListWrap.style.display = lines.length ? "grid" : "none";
}

function applyProgress(item: CheckResult) {
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
    bindErrorCopy(time);
    bindErrorCopy(row);
    const serverCell = dnsRows.querySelector<HTMLElement>(`[data-line="${item.line}"] .server-cell`);
    successfulServers.set(item.line, `${serverCell?.textContent?.trim() || item.server}    # line ${item.line}`);
  } else {
    row.textContent = errorLabel(item.error);
    bindErrorCopy(time, item.error);
    bindErrorCopy(row, item.error);
    successfulServers.delete(item.line);
  }

  updateSuccessList();
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
    concurrency: String(data.get("concurrency") || "32"),
    runId: activeRunId,
  };
}

renderPending("");

listen<CheckProgressEvent>("dns-check-progress", (event) => {
  if (event.payload.runId !== activeRunId) return;
  applyProgress(event.payload.result);
});

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  activeRunId = `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  const values = getFormValues();
  const total = values.servers
    .split("\n")
    .filter((line) => isCheckableLine(line)).length;
  checkButton.disabled = true;
  checkButton.textContent = "检测中...";
  resetRunState(total);
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
    renderGlobalError(String(error));
  } finally {
    checkButton.disabled = false;
  }
});
