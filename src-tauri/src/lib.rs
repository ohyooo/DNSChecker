use hickory_resolver::{
    config::{ConnectionConfig, NameServerConfig, ProtocolConfig, ResolverConfig, ResolverOpts},
    net::runtime::TokioRuntimeProvider,
    proto::rr::{RData, RecordType},
    TokioResolver,
};
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, SocketAddr},
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{sync::Semaphore, task::JoinSet, time::timeout as tokio_timeout};

#[derive(Debug, Deserialize, Serialize, Clone)]
struct CheckResult {
    #[serde(default)]
    line: u32,
    server: String,
    protocol: Option<String>,
    domain: String,
    #[serde(rename = "type")]
    type_name: String,
    #[serde(default)]
    duration_ms: f64,
    #[serde(default)]
    answers: Vec<String>,
    #[serde(default)]
    expected: Vec<String>,
    matched: Option<bool>,
    status: String,
    error: Option<String>,
    response_code: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct BatchCheckResponse {
    results: Vec<CheckResult>,
    total: u32,
    ok: u32,
    failed: u32,
}

#[derive(Debug, Deserialize, Serialize)]
struct ExpandServersResponse {
    servers: String,
    changed: bool,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SingleCheckRequest {
    line: u32,
    server: String,
    domain: String,
    type_name: String,
    expected: String,
    bootstrap: String,
    timeout: String,
}

#[derive(Debug, Clone)]
struct ParsedServer {
    line: u32,
    raw: String,
    protocol: String,
    host: String,
    port: u16,
    path: Option<String>,
    bootstrap: Vec<IpAddr>,
}

#[tauri::command]
async fn check_servers(
    servers: String,
    domain: String,
    type_name: String,
    expected: String,
    bootstrap: String,
    timeout: String,
    concurrency: String,
) -> Result<BatchCheckResponse, String> {
    let timeout = parse_timeout(&timeout)?;
    let concurrency = parse_concurrency(&concurrency);
    let global_bootstrap = parse_bootstrap_list(&bootstrap)?;
    let expected_values = parse_expected(&expected);
    let record_type = parse_record_type(&type_name)?;
    let parsed_servers = parse_servers(&servers, &global_bootstrap)?;

    let total = parsed_servers.len() as u32;
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut tasks = JoinSet::new();

    for server in parsed_servers {
        let semaphore = Arc::clone(&semaphore);
        let domain = domain.clone();
        let type_name = type_name.clone();
        let expected_values = expected_values.clone();
        tasks.spawn(async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .map_err(|err| format!("failed to acquire semaphore: {err}"))?;
            Ok::<CheckResult, String>(
                check_single_server(
                    server,
                    domain,
                    type_name,
                    expected_values,
                    record_type,
                    timeout,
                )
                .await,
            )
        });
    }

    let mut results = Vec::with_capacity(total as usize);
    let mut ok = 0u32;

    while let Some(joined) = tasks.join_next().await {
        let result = joined.map_err(|err| format!("DNS task join failed: {err}"))??;
        if result.status == "ok" {
            ok += 1;
        }
        results.push(result);
    }

    results.sort_by_key(|item| item.line);
    Ok(BatchCheckResponse {
        failed: total.saturating_sub(ok),
        ok,
        total,
        results,
    })
}

#[tauri::command]
async fn expand_servers(
    servers: String,
    bootstrap: String,
    timeout: String,
    domain: String,
    type_name: String,
    expected: String,
    concurrency: String,
) -> Result<ExpandServersResponse, String> {
    let _ = (bootstrap, timeout, domain, type_name, expected, concurrency);
    Ok(ExpandServersResponse {
        servers,
        changed: false,
        error: None,
    })
}

#[tauri::command]
async fn check_server(request: SingleCheckRequest) -> Result<CheckResult, String> {
    let timeout = parse_timeout(&request.timeout)?;
    let global_bootstrap = parse_bootstrap_list(&request.bootstrap)?;
    let expected_values = parse_expected(&request.expected);
    let record_type = parse_record_type(&request.type_name)?;
    let server = parse_server_line(request.line, request.server.trim(), &global_bootstrap)?;

    Ok(check_single_server(
        server,
        request.domain,
        request.type_name,
        expected_values,
        record_type,
        timeout,
    )
    .await)
}

async fn check_single_server(
    server: ParsedServer,
    domain: String,
    type_name: String,
    expected_values: Vec<String>,
    record_type: RecordType,
    timeout: Duration,
) -> CheckResult {
    let started = Instant::now();
    match lookup_server(&server, &domain, record_type, timeout).await {
        Ok(answers) => {
            let matched = matches_expected(&answers, &expected_values);
            let status = if matched { "ok" } else { "error" }.to_string();
            let error = if matched {
                None
            } else {
                Some(format!(
                    "expected [{}], got [{}]",
                    expected_values.join(", "),
                    answers.join(", ")
                ))
            };
            CheckResult {
                line: server.line,
                server: server.raw,
                protocol: Some(server.protocol),
                domain,
                type_name,
                duration_ms: started.elapsed().as_secs_f64() * 1000.0,
                answers,
                expected: expected_values,
                matched: Some(matched),
                status,
                error,
                response_code: None,
            }
        }
        Err(error) => CheckResult {
            matched: if expected_values.is_empty() {
                None
            } else {
                Some(false)
            },
            line: server.line,
            server: server.raw,
            protocol: Some(server.protocol),
            domain,
            type_name,
            duration_ms: started.elapsed().as_secs_f64() * 1000.0,
            answers: Vec::new(),
            expected: expected_values,
            status: "error".to_string(),
            error: Some(error),
            response_code: None,
        },
    }
}

async fn lookup_server(
    server: &ParsedServer,
    domain: &str,
    record_type: RecordType,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let socket_addrs = resolve_socket_addrs(server).await?;
    let mut errors = Vec::new();

    for socket_addr in socket_addrs {
        match lookup_socket_addr(server, socket_addr, domain, record_type, timeout).await {
            Ok(answers) => return Ok(answers),
            Err(error) => errors.push(format!("{socket_addr}: {error}")),
        }
    }

    Err(format!(
        "all upstream addresses failed for {}: {}",
        server.host,
        errors.join("; ")
    ))
}

async fn lookup_socket_addr(
    server: &ParsedServer,
    socket_addr: SocketAddr,
    domain: &str,
    record_type: RecordType,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let resolver = TokioResolver::builder_with_config(
        ResolverConfig::from_parts(
            None,
            vec![],
            vec![name_server_config(
                server,
                socket_addr.port(),
                socket_addr.ip(),
            )?],
        ),
        TokioRuntimeProvider::default(),
    )
    .with_options(resolver_opts(timeout))
    .build()
    .map_err(|err| err.to_string())?;

    let lookup = tokio_timeout(timeout, resolver.lookup(domain, record_type))
        .await
        .map_err(|_| {
            format!(
                "lookup timed out after {}",
                humantime::format_duration(timeout)
            )
        })?
        .map_err(|err| err.to_string())?;

    let mut answers = Vec::new();
    for record in lookup.answers() {
        answers.push(format_rdata(&record.data));
    }
    answers.sort();
    answers.dedup();
    Ok(answers)
}

async fn resolve_socket_addrs(server: &ParsedServer) -> Result<Vec<SocketAddr>, String> {
    let socket_addrs = if let Ok(ip) = server.host.parse::<IpAddr>() {
        vec![SocketAddr::new(ip, server.port)]
    } else if !server.bootstrap.is_empty() {
        server
            .bootstrap
            .iter()
            .map(|ip| SocketAddr::new(*ip, server.port))
            .collect()
    } else {
        tokio::net::lookup_host((server.host.as_str(), server.port))
            .await
            .map_err(|err| format!("failed to resolve {}: {err}", server.host))?
            .collect()
    };

    if socket_addrs.is_empty() {
        return Err(format!("no address resolved for {}", server.host));
    }

    Ok(socket_addrs)
}

fn name_server_config(
    server: &ParsedServer,
    port: u16,
    ip: IpAddr,
) -> Result<NameServerConfig, String> {
    Ok(NameServerConfig::new(
        ip,
        false,
        vec![connection_config(server, port)?],
    ))
}

fn resolver_opts(timeout: Duration) -> ResolverOpts {
    let mut options = ResolverOpts::default();
    options.timeout = timeout;
    options.attempts = 1;
    options.num_concurrent_reqs = 1;
    options
}

fn format_rdata(data: &RData) -> String {
    match data {
        RData::A(value) => value.to_string(),
        RData::AAAA(value) => value.to_string(),
        RData::CNAME(value) => value.to_utf8(),
        RData::MX(value) => value.exchange.to_utf8(),
        RData::NS(value) => value.to_utf8(),
        RData::TXT(value) => value
            .txt_data
            .iter()
            .map(|bytes| String::from_utf8_lossy(bytes).to_string())
            .collect::<Vec<_>>()
            .join(" "),
        _ => data.to_string(),
    }
}

fn parse_servers(input: &str, global_bootstrap: &[IpAddr]) -> Result<Vec<ParsedServer>, String> {
    let mut items = Vec::new();
    for (index, line) in input.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        items.push(parse_server_line(
            (index + 1) as u32,
            trimmed,
            global_bootstrap,
        )?);
    }
    Ok(items)
}

fn parse_server_line(
    line: u32,
    input: &str,
    global_bootstrap: &[IpAddr],
) -> Result<ParsedServer, String> {
    let mut parts = input.split_whitespace();
    let endpoint = parts
        .next()
        .ok_or_else(|| format!("line {line}: empty server definition"))?;
    let inline_bootstrap = parts
        .map(parse_ip_addr)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("line {line}: {err}"))?;
    let bootstrap = if inline_bootstrap.is_empty() {
        global_bootstrap.to_vec()
    } else {
        inline_bootstrap
    };

    let (protocol, host, port, path) = if let Some(rest) = endpoint.strip_prefix("udp://") {
        parse_host_port("udp", rest, 53)?
    } else if let Some(rest) = endpoint.strip_prefix("tcp://") {
        parse_host_port("tcp", rest, 53)?
    } else if let Some(rest) = endpoint.strip_prefix("dot://") {
        parse_host_port("tls", rest, 853)?
    } else if let Some(rest) = endpoint.strip_prefix("tls://") {
        parse_host_port("tls", rest, 853)?
    } else if let Some(rest) = endpoint.strip_prefix("https://") {
        parse_https_endpoint("https", rest)?
    } else if let Some(rest) = endpoint.strip_prefix("doh://") {
        parse_https_endpoint("https", rest)?
    } else {
        parse_host_port("udp", endpoint, 53)?
    };

    Ok(ParsedServer {
        line,
        raw: input.to_string(),
        protocol: protocol.to_string(),
        host,
        port,
        path,
        bootstrap,
    })
}

fn parse_host_port(
    protocol: &'static str,
    value: &str,
    default_port: u16,
) -> Result<(&'static str, String, u16, Option<String>), String> {
    if value.is_empty() {
        return Err("missing host".to_string());
    }
    if let Ok(socket_addr) = value.parse::<SocketAddr>() {
        return Ok((
            protocol,
            socket_addr.ip().to_string(),
            socket_addr.port(),
            None,
        ));
    }
    if let Ok(ip) = value.parse::<IpAddr>() {
        return Ok((protocol, ip.to_string(), default_port, None));
    }

    let bracketless = value.trim_matches(['[', ']']);
    if let Some((host, port)) = split_host_port(bracketless) {
        return Ok((protocol, host.to_string(), port, None));
    }
    Ok((protocol, bracketless.to_string(), default_port, None))
}

fn parse_https_endpoint(
    protocol: &'static str,
    value: &str,
) -> Result<(&'static str, String, u16, Option<String>), String> {
    let slash_index = value.find('/').unwrap_or(value.len());
    let authority = &value[..slash_index];
    let path = if slash_index < value.len() {
        &value[slash_index..]
    } else {
        "/dns-query"
    };
    let (_, host, port, _) = parse_host_port(protocol, authority, 443)?;
    Ok((protocol, host, port, Some(path.to_string())))
}

fn split_host_port(value: &str) -> Option<(&str, u16)> {
    let (host, port) = value.rsplit_once(':')?;
    let port = port.parse::<u16>().ok()?;
    Some((host, port))
}

fn parse_ip_addr(value: &str) -> Result<IpAddr, String> {
    let normalized = value.trim().trim_matches(['[', ']']);
    IpAddr::from_str(normalized).map_err(|_| format!("invalid bootstrap IP: {value}"))
}

fn parse_bootstrap_list(input: &str) -> Result<Vec<IpAddr>, String> {
    input
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|item| !item.is_empty())
        .map(parse_ip_addr)
        .collect()
}

fn parse_expected(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn matches_expected(answers: &[String], expected: &[String]) -> bool {
    if expected.is_empty() {
        return true;
    }
    expected.iter().all(|item| {
        answers
            .iter()
            .any(|answer| answer.eq_ignore_ascii_case(item))
    })
}

fn parse_timeout(input: &str) -> Result<Duration, String> {
    humantime::parse_duration(input.trim())
        .map_err(|err| format!("invalid timeout '{input}': {err}"))
}

fn parse_concurrency(input: &str) -> usize {
    input
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .unwrap_or(32)
        .min(256)
}

fn parse_record_type(input: &str) -> Result<RecordType, String> {
    RecordType::from_str(input.trim()).map_err(|_| format!("unsupported record type: {input}"))
}

fn connection_config(server: &ParsedServer, port: u16) -> Result<ConnectionConfig, String> {
    let protocol = protocol_config(server)?;
    let mut config = ConnectionConfig::new(protocol);
    config.port = port;
    Ok(config)
}

fn protocol_config(server: &ParsedServer) -> Result<ProtocolConfig, String> {
    match server.protocol.as_str() {
        "udp" => Ok(ProtocolConfig::Udp),
        "tcp" => Ok(ProtocolConfig::Tcp),
        "tls" => Ok(ProtocolConfig::Tls {
            server_name: Arc::from(server.host.as_str()),
        }),
        "https" => Ok(ProtocolConfig::Https {
            server_name: Arc::from(server.host.as_str()),
            path: Arc::from(server.path.as_deref().unwrap_or("/dns-query")),
        }),
        other => Err(format!("unsupported protocol: {other}")),
    }
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            check_server,
            check_servers,
            expand_servers
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
