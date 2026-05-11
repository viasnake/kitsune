use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    InvalidArgument(String),
    Inventory(String),
    Runtime(String),
    Rcon(String),
    Safety(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(error) => write!(f, "{error}"),
            Error::InvalidArgument(message) => write!(f, "{message}"),
            Error::Inventory(message) => write!(f, "{message}"),
            Error::Runtime(message) => write!(f, "{message}"),
            Error::Rcon(message) => write!(f, "{message}"),
            Error::Safety(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Error::Io(value)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    pub nodes: Vec<Node>,
    pub instances: Vec<Instance>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub address: Option<String>,
    pub default_runtime: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Instance {
    pub name: String,
    pub role: Option<String>,
    pub node: Option<String>,
    pub runtime: Option<String>,
    pub paths: InstancePaths,
    pub rcon: RconConfig,
    pub logs: LogConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstancePaths {
    pub root: Option<PathBuf>,
    pub live: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RconConfig {
    pub enabled: bool,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub secret_ref: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogConfig {
    pub journald_unit: Option<String>,
}

impl Inventory {
    pub fn load(path: &Path) -> Result<Self> {
        let source = fs::read_to_string(path)?;
        parse_inventory(&source)
    }

    pub fn resolve(&self, target: &str) -> Result<&Instance> {
        self.instances
            .iter()
            .find(|instance| instance.name == target)
            .ok_or_else(|| Error::Inventory(format!("target not found in inventory: {target}")))
    }
}

pub fn parse_inventory(source: &str) -> Result<Inventory> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Section {
        None,
        Nodes,
        Instances,
    }

    let mut inventory = Inventory::default();
    let mut section = Section::None;
    let mut current_node: Option<Node> = None;
    let mut current_instance: Option<Instance> = None;
    let mut nested = String::new();

    for (index, raw_line) in source.lines().enumerate() {
        let line_without_comment = raw_line.split_once('#').map_or(raw_line, |(line, _)| line);
        if line_without_comment.trim().is_empty() {
            continue;
        }

        let indent = line_without_comment
            .chars()
            .take_while(|character| *character == ' ')
            .count();
        let line = line_without_comment.trim();

        if indent == 0 {
            if let Some(node) = current_node.take() {
                inventory.nodes.push(node);
            }
            if let Some(instance) = current_instance.take() {
                inventory.instances.push(instance);
            }
            nested.clear();
            section = match line {
                "nodes:" => Section::Nodes,
                "instances:" => Section::Instances,
                _ => Section::None,
            };
            continue;
        }

        match section {
            Section::Nodes => {
                if let Some(rest) = line.strip_prefix("- ") {
                    if let Some(node) = current_node.take() {
                        inventory.nodes.push(node);
                    }
                    current_node = Some(Node::default());
                    set_node_field(current_node.as_mut().expect("node is set"), rest, index + 1)?;
                } else if let Some(node) = current_node.as_mut() {
                    set_node_field(node, line, index + 1)?;
                }
            }
            Section::Instances => {
                if let Some(rest) = line.strip_prefix("- ") {
                    if let Some(instance) = current_instance.take() {
                        inventory.instances.push(instance);
                    }
                    nested.clear();
                    current_instance = Some(Instance::default());
                    set_instance_field(
                        current_instance.as_mut().expect("instance is set"),
                        "",
                        rest,
                        index + 1,
                    )?;
                } else if line.ends_with(':') && indent <= 4 {
                    nested = line.trim_end_matches(':').to_string();
                } else if let Some(instance) = current_instance.as_mut() {
                    set_instance_field(instance, &nested, line, index + 1)?;
                }
            }
            Section::None => {}
        }
    }

    if let Some(node) = current_node.take() {
        inventory.nodes.push(node);
    }
    if let Some(instance) = current_instance.take() {
        inventory.instances.push(instance);
    }

    Ok(inventory)
}

fn set_node_field(node: &mut Node, line: &str, line_number: usize) -> Result<()> {
    let (key, value) = split_key_value(line, line_number)?;
    match key {
        "name" => node.name = value.to_string(),
        "address" => node.address = Some(value.to_string()),
        "default_runtime" => node.default_runtime = Some(value.to_string()),
        _ => {}
    }
    Ok(())
}

fn set_instance_field(
    instance: &mut Instance,
    nested: &str,
    line: &str,
    line_number: usize,
) -> Result<()> {
    let (key, value) = split_key_value(line, line_number)?;
    match (nested, key) {
        ("", "name") => instance.name = value.to_string(),
        ("", "role") => instance.role = Some(value.to_string()),
        ("", "node") => instance.node = Some(value.to_string()),
        ("", "runtime") => instance.runtime = Some(value.to_string()),
        ("paths", "root") => instance.paths.root = Some(PathBuf::from(value)),
        ("paths", "live") => instance.paths.live = Some(PathBuf::from(value)),
        ("rcon", "enabled") => instance.rcon.enabled = parse_bool(value),
        ("rcon", "host") => instance.rcon.host = Some(value.to_string()),
        ("rcon", "port") => {
            instance.rcon.port = Some(value.parse::<u16>().map_err(|_| {
                Error::Inventory(format!("invalid rcon port at line {line_number}: {value}"))
            })?)
        }
        ("rcon", "secret_ref") => instance.rcon.secret_ref = Some(value.to_string()),
        ("logs", "journald_unit") => instance.logs.journald_unit = Some(value.to_string()),
        _ => {}
    }
    Ok(())
}

fn split_key_value(line: &str, line_number: usize) -> Result<(&str, &str)> {
    let (key, value) = line
        .split_once(':')
        .ok_or_else(|| Error::Inventory(format!("invalid inventory line {line_number}: {line}")))?;
    Ok((key.trim(), unquote(value.trim())))
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value)
}

fn parse_bool(value: &str) -> bool {
    matches!(value, "true" | "yes" | "on" | "1")
}

pub mod cli {
    use super::*;

    pub fn run(args: Vec<String>) -> Result<()> {
        let mut parser = ArgParser::new(args);
        if parser.peek().is_none() {
            print_help();
            return Ok(());
        }

        let inventory_path = parser.global_inventory_path();
        let command = parser
            .next()
            .ok_or_else(|| Error::InvalidArgument("missing command".to_string()))?;

        match command.as_str() {
            "status" => {
                let inventory = load_inventory(&inventory_path)?;
                let target = parser.next();
                status(&inventory, target.as_deref())
            }
            "logs" => {
                let inventory = load_inventory(&inventory_path)?;
                let target = parser.required("target")?;
                let mut lines = 100;
                let mut follow = false;
                while let Some(arg) = parser.next() {
                    match arg.as_str() {
                        "--lines" => {
                            lines =
                                parser.required("line count")?.parse::<u16>().map_err(|_| {
                                    Error::InvalidArgument("--lines must be a number".to_string())
                                })?;
                        }
                        "--follow" | "-f" => follow = true,
                        _ => {
                            return Err(Error::InvalidArgument(format!(
                                "unknown logs argument: {arg}"
                            )));
                        }
                    }
                }
                logs(&inventory, &target, lines, follow)
            }
            "start" | "stop" | "restart" => {
                let inventory = load_inventory(&inventory_path)?;
                let target = parser.required("target")?;
                lifecycle(&inventory, command.as_str(), &target)
            }
            "cmd" => {
                let inventory = load_inventory(&inventory_path)?;
                let target = parser.required("target")?;
                parser.consume_double_dash();
                let game_command = parser.rest_joined();
                if game_command.is_empty() {
                    return Err(Error::InvalidArgument("missing game command".to_string()));
                }
                cmd(&inventory, &target, &game_command)
            }
            "dev" => dev(parser),
            "plugin" => {
                let inventory = load_inventory(&inventory_path)?;
                plugin(parser, &inventory)
            }
            "config" => {
                let inventory = load_inventory(&inventory_path)?;
                config(parser, &inventory)
            }
            "backup" => {
                let inventory = load_inventory(&inventory_path)?;
                backup(parser, &inventory)
            }
            "maintenance" => {
                let inventory = load_inventory(&inventory_path)?;
                maintenance(parser, &inventory)
            }
            "materialize" => {
                let inventory = load_inventory(&inventory_path)?;
                let target = parser.required("target")?;
                materialize(&inventory, &target)
            }
            "--help" | "-h" | "help" => {
                print_help();
                Ok(())
            }
            _ => Err(Error::InvalidArgument(format!(
                "unknown command: {command}"
            ))),
        }
    }

    fn load_inventory(explicit_path: &Option<PathBuf>) -> Result<Inventory> {
        let path = if let Some(path) = explicit_path {
            path.clone()
        } else if let Ok(path) = env::var("KITSUNEBI_INVENTORY") {
            PathBuf::from(path)
        } else if Path::new("inventory/production.yaml").exists() {
            PathBuf::from("inventory/production.yaml")
        } else {
            PathBuf::from("inventory/development.yaml")
        };
        Inventory::load(&path)
    }

    fn status(inventory: &Inventory, target: Option<&str>) -> Result<()> {
        if let Some(target) = target {
            let instance = inventory.resolve(target)?;
            let status = runtime_status(instance)?;
            println!(
                "{}\truntime={}\tunit={}\tstate={}",
                instance.name,
                instance.runtime.as_deref().unwrap_or("systemd-java"),
                status.unit,
                status.state
            );
            return Ok(());
        }

        for instance in &inventory.instances {
            let status = runtime_status(instance)?;
            println!(
                "{}\truntime={}\tunit={}\tstate={}",
                instance.name,
                instance.runtime.as_deref().unwrap_or("systemd-java"),
                status.unit,
                status.state
            );
        }
        Ok(())
    }

    fn logs(inventory: &Inventory, target: &str, lines: u16, follow: bool) -> Result<()> {
        let instance = inventory.resolve(target)?;
        runtime_logs(instance, lines, follow)
    }

    fn lifecycle(inventory: &Inventory, operation: &str, target: &str) -> Result<()> {
        let instance = inventory.resolve(target)?;
        let start = Instant::now();
        let result = runtime_lifecycle(instance, operation);
        let duration_ms = start.elapsed().as_millis();
        let event = OperationEvent::operation(
            target,
            instance.runtime.as_deref().unwrap_or("systemd-java"),
            operation,
            result.is_ok(),
            duration_ms,
        );
        let _ = write_operation_event(&event);
        result
    }

    fn cmd(inventory: &Inventory, target: &str, game_command: &str) -> Result<()> {
        let instance = inventory.resolve(target)?;
        let start = Instant::now();
        let sender = RconCommandSender::new();
        let result = sender.send_command(instance, game_command);
        let duration_ms = start.elapsed().as_millis();
        let event = OperationEvent::command(
            target,
            instance.runtime.as_deref().unwrap_or("rcon"),
            game_command,
            result.is_ok(),
            duration_ms,
        );
        let _ = write_operation_event(&event);
        match result {
            Ok(response) => {
                if !response.trim().is_empty() {
                    println!("{}", response.trim_end());
                }
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn dev(mut parser: ArgParser) -> Result<()> {
        let command = parser.required("dev command")?;
        let compose_file = env::var("KITSUNEBI_DEV_COMPOSE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("templates/docker-compose/dev-stack.yml"));
        match command.as_str() {
            "up" => run_dev_operation(
                "up",
                Command::new("docker")
                    .arg("compose")
                    .arg("-f")
                    .arg(compose_file)
                    .arg("up")
                    .arg("-d"),
            ),
            "down" => run_dev_operation(
                "down",
                Command::new("docker")
                    .arg("compose")
                    .arg("-f")
                    .arg(compose_file)
                    .arg("down"),
            ),
            "reset" => run_dev_operation(
                "reset",
                Command::new("docker")
                    .arg("compose")
                    .arg("-f")
                    .arg(compose_file)
                    .arg("down")
                    .arg("-v"),
            ),
            "logs" => {
                let target = parser.required("target")?;
                run_dev_operation(
                    "logs",
                    Command::new("docker")
                        .arg("compose")
                        .arg("-f")
                        .arg(compose_file)
                        .arg("logs")
                        .arg("-f")
                        .arg(target),
                )
            }
            "cmd" => {
                let target = parser.required("target")?;
                parser.consume_double_dash();
                let game_command = parser.rest_joined();
                if game_command.is_empty() {
                    return Err(Error::InvalidArgument("missing game command".to_string()));
                }
                run_dev_operation(
                    "cmd",
                    Command::new("docker")
                        .arg("compose")
                        .arg("-f")
                        .arg(compose_file)
                        .arg("exec")
                        .arg("-T")
                        .arg(target)
                        .arg("rcon-cli")
                        .arg(game_command),
                )
            }
            _ => Err(Error::InvalidArgument(format!(
                "unknown dev command: {command}"
            ))),
        }
    }

    fn plugin(mut parser: ArgParser, inventory: &Inventory) -> Result<()> {
        let command = parser.required("plugin command")?;
        let manager = PluginManager::new();
        match command.as_str() {
            "diff" => {
                let target = parser.required("target")?;
                let instance = inventory.resolve(&target)?;
                manager.diff(instance)
            }
            "sync" => {
                let target = parser.required("target")?;
                let instance = inventory.resolve(&target)?;
                let start = Instant::now();
                let result = manager.sync(instance);
                let event = OperationEvent::operation(
                    &target,
                    instance.runtime.as_deref().unwrap_or("manual"),
                    "plugin.sync",
                    result.is_ok(),
                    start.elapsed().as_millis(),
                );
                let _ = write_operation_event(&event);
                result
            }
            "lock" => {
                let start = Instant::now();
                let result = manager.lock(inventory);
                let event = OperationEvent::operation(
                    "plugins",
                    "manual",
                    "plugin.lock",
                    result.is_ok(),
                    start.elapsed().as_millis(),
                );
                let _ = write_operation_event(&event);
                result
            }
            "update-plan" => {
                let plugin = parser.required("plugin")?;
                let mut to_version = None;
                while let Some(arg) = parser.next() {
                    match arg.as_str() {
                        "--to" => to_version = Some(parser.required("version")?),
                        _ => {
                            return Err(Error::InvalidArgument(format!(
                                "unknown plugin update-plan argument: {arg}"
                            )));
                        }
                    }
                }
                let start = Instant::now();
                let result = manager.update_plan(inventory, &plugin, to_version.as_deref());
                let event = OperationEvent::operation(
                    &plugin,
                    "manual",
                    "plugin.update-plan",
                    result.is_ok(),
                    start.elapsed().as_millis(),
                );
                let _ = write_operation_event(&event);
                result
            }
            "three-way-diff" => {
                let target = parser.required("target")?;
                let relative = parser.required("live relative path")?;
                let migrated = parser.required("migrated file")?;
                let instance = inventory.resolve(&target)?;
                manager.three_way_diff(instance, Path::new(&relative), Path::new(&migrated))
            }
            _ => Err(Error::InvalidArgument(format!(
                "unknown plugin command: {command}"
            ))),
        }
    }

    fn config(mut parser: ArgParser, inventory: &Inventory) -> Result<()> {
        let command = parser.required("config command")?;
        let target = parser.required("target")?;
        let instance = inventory.resolve(&target)?;
        let manager = ConfigManager::new();
        match command.as_str() {
            "diff" | "drift" => manager.diff(instance),
            "apply" => {
                let mut overwrite_conflicts = false;
                while let Some(arg) = parser.next() {
                    match arg.as_str() {
                        "--overwrite-conflicts" => overwrite_conflicts = true,
                        _ => {
                            return Err(Error::InvalidArgument(format!(
                                "unknown config apply argument: {arg}"
                            )));
                        }
                    }
                }
                let start = Instant::now();
                let result = manager.apply(instance, overwrite_conflicts);
                let event = OperationEvent::operation(
                    &target,
                    instance.runtime.as_deref().unwrap_or("manual"),
                    "config.apply",
                    result.is_ok(),
                    start.elapsed().as_millis(),
                );
                let _ = write_operation_event(&event);
                result
            }
            "import" => {
                let path = parser.required("live relative path")?;
                let start = Instant::now();
                let result = manager.import(instance, Path::new(&path));
                let event = OperationEvent::operation(
                    &target,
                    instance.runtime.as_deref().unwrap_or("manual"),
                    "config.import",
                    result.is_ok(),
                    start.elapsed().as_millis(),
                );
                let _ = write_operation_event(&event);
                result
            }
            _ => Err(Error::InvalidArgument(format!(
                "unknown config command: {command}"
            ))),
        }
    }

    fn backup(mut parser: ArgParser, inventory: &Inventory) -> Result<()> {
        let command = parser.required("backup command")?;
        match command.as_str() {
            "preflight" => {
                let target = parser.required("target")?;
                let instance = inventory.resolve(&target)?;
                BackupManager::new().preflight(instance)
            }
            _ => Err(Error::InvalidArgument(format!(
                "unknown backup command: {command}"
            ))),
        }
    }

    fn maintenance(mut parser: ArgParser, inventory: &Inventory) -> Result<()> {
        let command = parser.required("maintenance command")?;
        match command.as_str() {
            "restart" => {
                let target = parser.required("target")?;
                let mut notice = None;
                let mut confirm = false;
                while let Some(arg) = parser.next() {
                    match arg.as_str() {
                        "--notice" => notice = Some(parser.required("notice")?),
                        "--confirm" => confirm = true,
                        _ => {
                            return Err(Error::InvalidArgument(format!(
                                "unknown maintenance restart argument: {arg}"
                            )));
                        }
                    }
                }
                if !confirm {
                    return Err(Error::Safety(
                        "maintenance restart requires --confirm".to_string(),
                    ));
                }
                let instance = inventory.resolve(&target)?;
                MaintenanceManager::new().restart(instance, notice.as_deref())
            }
            _ => Err(Error::InvalidArgument(format!(
                "unknown maintenance command: {command}"
            ))),
        }
    }

    fn materialize(inventory: &Inventory, target: &str) -> Result<()> {
        let instance = inventory.resolve(target)?;
        let start = Instant::now();
        println!("Materialize: {target}");
        println!("1. inventory load: ok");
        println!("2. target resolve: ok");
        println!("3. plugin diff:");
        PluginManager::new().diff(instance)?;
        println!("4. config diff:");
        ConfigManager::new().diff(instance)?;
        println!("5. plugin sync:");
        PluginManager::new().sync(instance)?;
        println!("6. config apply:");
        ConfigManager::new().apply(instance, false)?;
        println!("7. runtime status:");
        let status = runtime_status(instance)?;
        println!("{} {} {}", status.unit, status.state, target);
        let event = OperationEvent::operation(
            target,
            instance.runtime.as_deref().unwrap_or("systemd-java"),
            "materialize",
            true,
            start.elapsed().as_millis(),
        );
        let _ = write_operation_event(&event);
        Ok(())
    }

    fn print_help() {
        println!(
            "kitsunebi\n\n\
Usage:\n  \
  kitsunebi [--inventory PATH] status [target]\n  \
  kitsunebi [--inventory PATH] start|stop|restart <target>\n  \
  kitsunebi [--inventory PATH] logs <target> [--lines N] [--follow]\n  \
  kitsunebi [--inventory PATH] cmd <target> -- \"<game command>\"\n  \
  kitsunebi dev up|down|reset\n  \
  kitsunebi dev logs <target>\n  \
  kitsunebi dev cmd <target> -- \"<game command>\"\n  \
  kitsunebi [--inventory PATH] plugin diff|sync <target>\n  \
  kitsunebi [--inventory PATH] plugin lock\n  \
  kitsunebi [--inventory PATH] plugin update-plan <plugin> --to <version>\n  \
  kitsunebi [--inventory PATH] plugin three-way-diff <target> <live-relative-path> <migrated-file>\n  \
  kitsunebi [--inventory PATH] config diff|drift <target>\n  \
  kitsunebi [--inventory PATH] config apply <target> [--overwrite-conflicts]\n  \
  kitsunebi [--inventory PATH] config import <target> <live-relative-path>\n  \
  kitsunebi [--inventory PATH] backup preflight <target>\n  \
  kitsunebi [--inventory PATH] maintenance restart <target> [--notice TEXT] --confirm\n  \
  kitsunebi [--inventory PATH] materialize <target>"
        );
    }

    struct ArgParser {
        args: Vec<String>,
        index: usize,
    }

    impl ArgParser {
        fn new(args: Vec<String>) -> Self {
            Self { args, index: 0 }
        }

        fn peek(&self) -> Option<&str> {
            self.args.get(self.index).map(String::as_str)
        }

        fn next(&mut self) -> Option<String> {
            let value = self.args.get(self.index).cloned();
            if value.is_some() {
                self.index += 1;
            }
            value
        }

        fn required(&mut self, name: &str) -> Result<String> {
            self.next()
                .ok_or_else(|| Error::InvalidArgument(format!("missing {name}")))
        }

        fn consume_double_dash(&mut self) {
            if self.peek() == Some("--") {
                self.index += 1;
            }
        }

        fn rest_joined(&mut self) -> String {
            let rest = self.args[self.index..].join(" ");
            self.index = self.args.len();
            rest
        }

        fn global_inventory_path(&mut self) -> Option<PathBuf> {
            let mut inventory_path = None;
            let mut normalized = Vec::new();
            let mut index = 0;
            while index < self.args.len() {
                if self.args[index] == "--inventory" {
                    if let Some(path) = self.args.get(index + 1) {
                        inventory_path = Some(PathBuf::from(path));
                        index += 2;
                        continue;
                    }
                }
                normalized.push(self.args[index].clone());
                index += 1;
            }
            self.args = normalized;
            self.index = 0;
            inventory_path
        }
    }
}

fn runtime_status(instance: &Instance) -> Result<Status> {
    match instance.runtime.as_deref() {
        Some("docker-compose") => DockerComposeAdapter::new().status(instance),
        _ => SystemdJavaAdapter::new().status(instance),
    }
}

fn runtime_logs(instance: &Instance, lines: u16, follow: bool) -> Result<()> {
    match instance.runtime.as_deref() {
        Some("docker-compose") => DockerComposeAdapter::new().logs(instance, lines, follow),
        _ => SystemdJavaAdapter::new().logs(instance, lines, follow),
    }
}

fn runtime_lifecycle(instance: &Instance, operation: &str) -> Result<()> {
    match instance.runtime.as_deref() {
        Some("docker-compose") => {
            let adapter = DockerComposeAdapter::new();
            match operation {
                "start" => adapter.start(instance),
                "stop" => adapter.stop(instance),
                "restart" => adapter.restart(instance),
                _ => Err(Error::InvalidArgument(format!(
                    "unsupported lifecycle operation: {operation}"
                ))),
            }
        }
        _ => {
            let adapter = SystemdJavaAdapter::new();
            match operation {
                "start" => adapter.start(instance),
                "stop" => adapter.stop(instance),
                "restart" => adapter.restart(instance),
                _ => Err(Error::InvalidArgument(format!(
                    "unsupported lifecycle operation: {operation}"
                ))),
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Status {
    pub unit: String,
    pub state: String,
}

pub trait RuntimeAdapter {
    fn status(&self, target: &Instance) -> Result<Status>;
    fn logs(&self, target: &Instance, lines: u16, follow: bool) -> Result<()>;
    fn start(&self, target: &Instance) -> Result<()>;
    fn stop(&self, target: &Instance) -> Result<()>;
    fn restart(&self, target: &Instance) -> Result<()>;
}

pub struct SystemdJavaAdapter;

impl SystemdJavaAdapter {
    pub fn new() -> Self {
        Self
    }

    fn unit(&self, target: &Instance) -> String {
        target
            .logs
            .journald_unit
            .clone()
            .unwrap_or_else(|| format!("kitsunebi@{}.service", target.name))
    }
}

impl Default for SystemdJavaAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeAdapter for SystemdJavaAdapter {
    fn status(&self, target: &Instance) -> Result<Status> {
        let unit = self.unit(target);
        let output = Command::new("systemctl")
            .arg("is-active")
            .arg(&unit)
            .output();
        let state = match output {
            Ok(output) if output.status.success() => {
                String::from_utf8_lossy(&output.stdout).trim().to_string()
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                if stderr.is_empty() {
                    "inactive".to_string()
                } else {
                    format!("unknown ({stderr})")
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                "unavailable (systemctl not found)".to_string()
            }
            Err(error) => return Err(Error::Runtime(error.to_string())),
        };
        Ok(Status { unit, state })
    }

    fn logs(&self, target: &Instance, lines: u16, follow: bool) -> Result<()> {
        let unit = self.unit(target);
        let mut command = Command::new("journalctl");
        command
            .arg("-u")
            .arg(unit)
            .arg("-n")
            .arg(lines.to_string())
            .arg("--no-pager");
        if follow {
            command.arg("-f");
        }
        run_status(&mut command)
    }

    fn start(&self, target: &Instance) -> Result<()> {
        run_status(
            Command::new("systemctl")
                .arg("start")
                .arg(self.unit(target)),
        )
    }

    fn stop(&self, target: &Instance) -> Result<()> {
        run_status(Command::new("systemctl").arg("stop").arg(self.unit(target)))
    }

    fn restart(&self, target: &Instance) -> Result<()> {
        run_status(
            Command::new("systemctl")
                .arg("restart")
                .arg(self.unit(target)),
        )
    }
}

pub struct DockerComposeAdapter {
    compose_file: PathBuf,
}

impl DockerComposeAdapter {
    pub fn new() -> Self {
        Self {
            compose_file: env::var("KITSUNEBI_DEV_COMPOSE")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("templates/docker-compose/dev-stack.yml")),
        }
    }

    fn service_name(&self, target: &Instance) -> String {
        target.name.clone()
    }
}

impl Default for DockerComposeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeAdapter for DockerComposeAdapter {
    fn status(&self, target: &Instance) -> Result<Status> {
        let service = self.service_name(target);
        let output = Command::new("docker")
            .arg("compose")
            .arg("-f")
            .arg(&self.compose_file)
            .arg("ps")
            .arg(&service)
            .output();
        let state = match output {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.contains(&service) {
                    "listed".to_string()
                } else {
                    "not-created".to_string()
                }
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                if stderr.is_empty() {
                    "unknown".to_string()
                } else {
                    format!("unknown ({stderr})")
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                "unavailable (docker not found)".to_string()
            }
            Err(error) => return Err(Error::Runtime(error.to_string())),
        };
        Ok(Status {
            unit: service,
            state,
        })
    }

    fn logs(&self, target: &Instance, lines: u16, follow: bool) -> Result<()> {
        let mut command = Command::new("docker");
        command
            .arg("compose")
            .arg("-f")
            .arg(&self.compose_file)
            .arg("logs")
            .arg("--tail")
            .arg(lines.to_string());
        if follow {
            command.arg("-f");
        }
        command.arg(self.service_name(target));
        run_status(&mut command)
    }

    fn start(&self, target: &Instance) -> Result<()> {
        run_status(
            Command::new("docker")
                .arg("compose")
                .arg("-f")
                .arg(&self.compose_file)
                .arg("up")
                .arg("-d")
                .arg(self.service_name(target)),
        )
    }

    fn stop(&self, target: &Instance) -> Result<()> {
        run_status(
            Command::new("docker")
                .arg("compose")
                .arg("-f")
                .arg(&self.compose_file)
                .arg("stop")
                .arg(self.service_name(target)),
        )
    }

    fn restart(&self, target: &Instance) -> Result<()> {
        run_status(
            Command::new("docker")
                .arg("compose")
                .arg("-f")
                .arg(&self.compose_file)
                .arg("restart")
                .arg(self.service_name(target)),
        )
    }
}

pub trait CommandSender {
    fn send_command(&self, target: &Instance, command: &str) -> Result<String>;
}

pub struct RconCommandSender {
    timeout: Duration,
}

impl RconCommandSender {
    pub fn new() -> Self {
        Self {
            timeout: Duration::from_secs(5),
        }
    }
}

impl Default for RconCommandSender {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandSender for RconCommandSender {
    fn send_command(&self, target: &Instance, command: &str) -> Result<String> {
        if !target.rcon.enabled {
            return Err(Error::Rcon(format!("RCON is disabled for {}", target.name)));
        }
        let host = target
            .rcon
            .host
            .as_deref()
            .unwrap_or("127.0.0.1")
            .to_string();
        let port = target.rcon.port.unwrap_or(25575);
        let password = read_rcon_password(target)?;
        let mut stream = TcpStream::connect((host.as_str(), port))
            .map_err(|error| Error::Rcon(format!("failed to connect to RCON: {error}")))?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;

        write_rcon_packet(&mut stream, 1, 3, &password)?;
        let auth = read_rcon_packet(&mut stream)?;
        if auth.request_id == -1 {
            return Err(Error::Rcon("RCON authentication failed".to_string()));
        }

        write_rcon_packet(&mut stream, 2, 2, command)?;
        let response = read_rcon_packet(&mut stream)?;
        Ok(response.payload)
    }
}

struct RconPacket {
    request_id: i32,
    payload: String,
}

fn write_rcon_packet(
    stream: &mut TcpStream,
    request_id: i32,
    packet_type: i32,
    payload: &str,
) -> Result<()> {
    let size = 4 + 4 + payload.len() + 2;
    let mut buffer = Vec::with_capacity(size + 4);
    buffer.extend_from_slice(&(size as i32).to_le_bytes());
    buffer.extend_from_slice(&request_id.to_le_bytes());
    buffer.extend_from_slice(&packet_type.to_le_bytes());
    buffer.extend_from_slice(payload.as_bytes());
    buffer.push(0);
    buffer.push(0);
    stream.write_all(&buffer)?;
    Ok(())
}

fn read_rcon_packet(stream: &mut TcpStream) -> Result<RconPacket> {
    let mut length_bytes = [0_u8; 4];
    stream.read_exact(&mut length_bytes)?;
    let length = i32::from_le_bytes(length_bytes);
    if !(10..=4_096).contains(&length) {
        return Err(Error::Rcon(format!("invalid RCON packet length: {length}")));
    }
    let mut buffer = vec![0_u8; length as usize];
    stream.read_exact(&mut buffer)?;
    let request_id = i32::from_le_bytes(buffer[0..4].try_into().expect("slice length"));
    let _packet_type = i32::from_le_bytes(buffer[4..8].try_into().expect("slice length"));
    let payload_end = buffer.len().saturating_sub(2);
    let payload = String::from_utf8_lossy(&buffer[8..payload_end]).to_string();
    Ok(RconPacket {
        request_id,
        payload,
    })
}

fn read_rcon_password(target: &Instance) -> Result<String> {
    if let Ok(password) = env::var("KITSUNEBI_RCON_PASSWORD") {
        return Ok(password);
    }

    let target_env_name = format!(
        "KITSUNEBI_{}_RCON_PASSWORD",
        target.name.replace('-', "_").to_ascii_uppercase()
    );
    if let Ok(password) = env::var(target_env_name) {
        return Ok(password);
    }

    for path in [
        PathBuf::from(format!("/etc/kitsunebi/secrets/{}.env", target.name)),
        PathBuf::from(format!("secrets/{}.env", target.name)),
        PathBuf::from("secrets/rcon.env"),
    ] {
        if !path.exists() {
            continue;
        }
        let content = fs::read_to_string(&path)?;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = trimmed.split_once('=') {
                if key == "RCON_PASSWORD" {
                    return Ok(unquote(value.trim()).to_string());
                }
            }
        }
    }

    Err(Error::Rcon(format!(
        "RCON password not found for {}; set KITSUNEBI_RCON_PASSWORD or /etc/kitsunebi/secrets/{}.env",
        target.name, target.name
    )))
}

#[derive(Debug)]
pub struct OperationEvent {
    event: String,
    actor: String,
    target: String,
    operation: String,
    runtime: String,
    command_hash: Option<String>,
    command_preview: Option<String>,
    result: String,
    duration_ms: u128,
}

impl OperationEvent {
    pub fn operation(
        target: &str,
        runtime: &str,
        operation: &str,
        success: bool,
        duration_ms: u128,
    ) -> Self {
        Self {
            event: format!("kitsunebi.{operation}"),
            actor: env::var("USER").unwrap_or_else(|_| "unknown".to_string()),
            target: target.to_string(),
            operation: operation.to_string(),
            runtime: runtime.to_string(),
            command_hash: None,
            command_preview: None,
            result: if success { "success" } else { "failure" }.to_string(),
            duration_ms,
        }
    }

    pub fn command(
        target: &str,
        runtime: &str,
        command: &str,
        success: bool,
        duration_ms: u128,
    ) -> Self {
        Self {
            event: "kitsunebi.command".to_string(),
            actor: env::var("USER").unwrap_or_else(|_| "unknown".to_string()),
            target: target.to_string(),
            operation: "cmd".to_string(),
            runtime: runtime.to_string(),
            command_hash: sha256_of_bytes(command.as_bytes()),
            command_preview: Some(mask_command_preview(command)),
            result: if success { "success" } else { "failure" }.to_string(),
            duration_ms,
        }
    }

    fn to_json(&self) -> String {
        let mut fields = vec![
            json_field("event", &self.event),
            json_field("actor", &self.actor),
            json_field("target", &self.target),
            json_field("operation", &self.operation),
            json_field("runtime", &self.runtime),
            json_field("result", &self.result),
            format!("\"duration_ms\":{}", self.duration_ms),
        ];
        if let Some(command_hash) = &self.command_hash {
            fields.push(json_field("command_hash", command_hash));
        }
        if let Some(command_preview) = &self.command_preview {
            fields.push(json_field("command_preview", command_preview));
        }
        format!("{{{}}}", fields.join(","))
    }
}

pub fn write_operation_event(event: &OperationEvent) -> Result<()> {
    let json = event.to_json();
    let mut child = Command::new("systemd-cat")
        .arg("-t")
        .arg("kitsunebi")
        .arg("-p")
        .arg("info")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    match child.as_mut() {
        Ok(child) => {
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(json.as_bytes())?;
                stdin.write_all(b"\n")?;
            }
            let _ = child.wait();
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("{json}");
        }
        Err(error) => return Err(Error::Runtime(error.to_string())),
    }
    Ok(())
}

fn run_dev_operation(operation: &str, command: &mut Command) -> Result<()> {
    let start = Instant::now();
    let result = run_status(command);
    let event = OperationEvent::operation(
        "development",
        "docker-compose",
        &format!("dev.{operation}"),
        result.is_ok(),
        start.elapsed().as_millis(),
    );
    let _ = write_operation_event(&event);
    result
}

fn json_field(key: &str, value: &str) -> String {
    format!("\"{}\":\"{}\"", json_escape(key), json_escape(value))
}

fn json_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

pub fn mask_command_preview(command: &str) -> String {
    let masked = command
        .split_whitespace()
        .map(|part| {
            let lowercase = part.to_ascii_lowercase();
            if lowercase.starts_with("password=")
                || lowercase.starts_with("token=")
                || lowercase.starts_with("secret=")
                || lowercase.starts_with("key=")
            {
                let key = part.split_once('=').map_or(part, |(key, _)| key);
                format!("{key}=***")
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if masked.len() > 120 {
        format!("{}...", &masked[..120])
    } else {
        masked
    }
}

fn sha256_of_bytes(bytes: &[u8]) -> Option<String> {
    let mut child = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(bytes).ok()?;
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let hash = text.split_whitespace().next()?;
    Some(format!("sha256:{hash}"))
}

pub struct PluginManager {
    repo_root: PathBuf,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            repo_root: PathBuf::from("."),
        }
    }

    pub fn diff(&self, target: &Instance) -> Result<()> {
        let desired_dir = self.desired_plugin_dir(target);
        let live_dir = target_live_path(target)?.join("plugins");
        let desired = list_jar_files(&desired_dir)?;
        let live = list_jar_files(&live_dir)?;
        println!("Plugin diff: {}", target.name);
        print_name_set("Desired jars", &desired);
        print_name_set("Live jars", &live);
        let missing = desired.difference(&live).cloned().collect::<BTreeSet<_>>();
        let unknown = live.difference(&desired).cloned().collect::<BTreeSet<_>>();
        print_name_set("Missing from live", &missing);
        print_name_set("Unknown live jars", &unknown);
        println!("No action taken.");
        Ok(())
    }

    pub fn sync(&self, target: &Instance) -> Result<()> {
        let desired_dir = self.desired_plugin_dir(target);
        let live_dir = target_live_path(target)?.join("plugins");
        fs::create_dir_all(&live_dir)?;
        let desired = list_jar_files(&desired_dir)?;
        println!("Plugin sync: {}", target.name);
        if desired.is_empty() {
            println!("No desired jars found in {}", desired_dir.display());
            return Ok(());
        }
        let snapshot_dir = target_runtime_path(target)
            .join("plugin-snapshots")
            .join(timestamp_label());
        for jar in desired {
            let source = desired_dir.join(&jar);
            let destination = live_dir.join(&jar);
            if destination.exists() {
                fs::create_dir_all(&snapshot_dir)?;
                fs::copy(&destination, snapshot_dir.join(&jar))?;
            }
            fs::copy(&source, &destination)?;
            println!("copied {jar}");
        }
        let unknown = list_jar_files(&live_dir)?
            .difference(&list_jar_files(&desired_dir)?)
            .cloned()
            .collect::<BTreeSet<_>>();
        print_name_set("Unknown live jars left untouched", &unknown);
        Ok(())
    }

    pub fn lock(&self, inventory: &Inventory) -> Result<()> {
        let mut entries = Vec::new();
        for instance in &inventory.instances {
            let desired_dir = self.desired_plugin_dir(instance);
            for jar in list_jar_files(&desired_dir)? {
                let path = desired_dir.join(&jar);
                let sha256 = sha256_of_file(&path)?;
                entries.push(PluginLockEntry {
                    key: format!("{}/{}", instance.name, jar_stem(&jar)),
                    instance: instance.name.clone(),
                    filename: jar,
                    path,
                    sha256,
                });
            }
        }
        entries.sort_by(|left, right| left.key.cmp(&right.key));
        let lock_path = self.repo_root.join("plugins").join("plugins.lock");
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = String::from("plugins:\n");
        if entries.is_empty() {
            output.push_str("  {}\n");
        } else {
            for entry in entries {
                output.push_str(&format!("  {}:\n", yaml_key(&entry.key)));
                output.push_str("    source: manual\n");
                output.push_str(&format!("    instance: {}\n", entry.instance));
                output.push_str(&format!("    filename: \"{}\"\n", entry.filename));
                output.push_str(&format!("    file: \"{}\"\n", normalize_path(&entry.path)));
                output.push_str(&format!("    sha256: \"{}\"\n", entry.sha256));
            }
        }
        fs::write(&lock_path, output)?;
        println!("wrote {}", lock_path.display());
        Ok(())
    }

    pub fn update_plan(
        &self,
        inventory: &Inventory,
        plugin: &str,
        to_version: Option<&str>,
    ) -> Result<()> {
        let target_version = to_version.unwrap_or("unspecified");
        println!("Plugin update plan: {plugin}");
        println!("Target version: {target_version}");
        println!();
        println!("Affected instances:");
        let mut affected = 0;
        for instance in &inventory.instances {
            let desired_dir = self.desired_plugin_dir(instance);
            let matched_jars = list_jar_files(&desired_dir)?
                .into_iter()
                .filter(|jar| {
                    jar.to_ascii_lowercase()
                        .contains(&plugin.to_ascii_lowercase())
                })
                .collect::<Vec<_>>();
            if matched_jars.is_empty() {
                continue;
            }
            affected += 1;
            println!("  {}", instance.name);
            for jar in matched_jars {
                let path = desired_dir.join(&jar);
                let hash = sha256_of_file(&path)?;
                println!("    current jar: {jar}");
                println!("    current sha256: {hash}");
            }
            let config_dir = ConfigManager::new()
                .source_config_dir(instance)
                .join("plugins")
                .join(plugin);
            if config_dir.exists() {
                println!("    managed config: {}", config_dir.display());
            }
            let policy_file = self
                .repo_root
                .join("instances")
                .join(&instance.name)
                .join("plugin-policy.yaml");
            if policy_file.exists() {
                println!("    policy: {}", policy_file.display());
                if file_contains_case_insensitive(&policy_file, "requires_external_db_snapshot")? {
                    println!("    risk: external DB snapshot may be required");
                }
            }
        }
        if affected == 0 {
            println!("  none");
        }
        println!();
        println!("Required manual steps:");
        println!(
            "  1. Add the candidate jar to plugins/manual/<instance>/ or instances/<instance>/plugins/."
        );
        println!("  2. Run kitsunebi plugin lock.");
        println!("  3. Run kitsunebi dev up and let the plugin migrate config/state if needed.");
        println!("  4. Compare repo config, live config before update, and migrated config.");
        println!("  5. Import only reviewed config changes with kitsunebi config import.");
        println!("  6. Take external DB snapshots for DB-backed plugins before production apply.");
        println!("No files changed.");
        Ok(())
    }

    pub fn three_way_diff(
        &self,
        target: &Instance,
        relative: &Path,
        migrated: &Path,
    ) -> Result<()> {
        validate_relative_path(relative)?;
        let config_manager = ConfigManager {
            repo_root: self.repo_root.clone(),
        };
        let repo_path = config_manager.source_config_dir(target).join(relative);
        let live_path = target_live_path(target)?.join(relative);
        if !repo_path.exists() {
            return Err(Error::InvalidArgument(format!(
                "repo managed config does not exist: {}",
                repo_path.display()
            )));
        }
        if !live_path.exists() {
            return Err(Error::InvalidArgument(format!(
                "live config does not exist: {}",
                live_path.display()
            )));
        }
        if !migrated.exists() {
            return Err(Error::InvalidArgument(format!(
                "migrated config does not exist: {}",
                migrated.display()
            )));
        }
        let repo_hash = sha256_of_file(&repo_path)?;
        let live_hash = sha256_of_file(&live_path)?;
        let migrated_hash = sha256_of_file(migrated)?;
        println!("Plugin config three-way diff: {}", target.name);
        println!("Path: {}", relative.display());
        println!("  repo managed: {repo_hash}");
        println!("  live before:  {live_hash}");
        println!("  migrated:     {migrated_hash}");
        println!("Findings:");
        println!(
            "  repo_vs_live: {}",
            if repo_hash == live_hash {
                "same"
            } else {
                "different"
            }
        );
        println!(
            "  live_vs_migrated: {}",
            if live_hash == migrated_hash {
                "same"
            } else {
                "different"
            }
        );
        println!(
            "  repo_vs_migrated: {}",
            if repo_hash == migrated_hash {
                "same"
            } else {
                "different"
            }
        );
        println!("No action taken.");
        Ok(())
    }

    fn desired_plugin_dir(&self, target: &Instance) -> PathBuf {
        let instance_dir = self
            .repo_root
            .join("instances")
            .join(&target.name)
            .join("plugins");
        if instance_dir.exists() {
            return instance_dir;
        }
        let manual_dir = self
            .repo_root
            .join("plugins")
            .join("manual")
            .join(&target.name);
        if manual_dir.exists() {
            return manual_dir;
        }
        instance_dir
    }
}

struct PluginLockEntry {
    key: String,
    instance: String,
    filename: String,
    path: PathBuf,
    sha256: String,
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ConfigManager {
    repo_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfigDriftState {
    MissingLive,
    Unchanged,
    RepoChanged,
    LiveChanged,
    Conflict,
    UntrackedLive,
}

impl fmt::Display for ConfigDriftState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigDriftState::MissingLive => write!(f, "missing-live"),
            ConfigDriftState::Unchanged => write!(f, "unchanged"),
            ConfigDriftState::RepoChanged => write!(f, "repo-changed"),
            ConfigDriftState::LiveChanged => write!(f, "live-drift"),
            ConfigDriftState::Conflict => write!(f, "conflict"),
            ConfigDriftState::UntrackedLive => write!(f, "untracked-live"),
        }
    }
}

#[derive(Debug, Clone)]
struct ConfigDrift {
    relative: PathBuf,
    source_hash: String,
    live_hash: Option<String>,
    state: ConfigDriftState,
}

impl ConfigManager {
    pub fn new() -> Self {
        Self {
            repo_root: PathBuf::from("."),
        }
    }

    pub fn diff(&self, target: &Instance) -> Result<()> {
        let source_dir = self.source_config_dir(target);
        let managed = self.drift_entries(target)?;
        println!("Config drift: {}", target.name);
        if managed.is_empty() {
            println!("No managed config files found in {}", source_dir.display());
            println!("No action taken.");
            return Ok(());
        }
        for entry in managed {
            println!("  {} {}", entry.state, entry.relative.display());
            println!("    repo hash: {}", entry.source_hash);
            println!(
                "    live hash: {}",
                entry.live_hash.as_deref().unwrap_or("missing")
            );
        }
        println!("No action taken.");
        Ok(())
    }

    pub fn apply(&self, target: &Instance, overwrite_conflicts: bool) -> Result<()> {
        let source_dir = self.source_config_dir(target);
        let live_dir = target_live_path(target)?;
        let drift_entries = self.drift_entries(target)?;
        if drift_entries.is_empty() {
            println!("No managed config files found in {}", source_dir.display());
            return Ok(());
        }
        let blocked = drift_entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry.state,
                    ConfigDriftState::LiveChanged
                        | ConfigDriftState::Conflict
                        | ConfigDriftState::UntrackedLive
                )
            })
            .collect::<Vec<_>>();
        if !blocked.is_empty() && !overwrite_conflicts {
            println!("Config apply blocked: {}", target.name);
            for entry in blocked {
                println!("  {} {}", entry.state, entry.relative.display());
            }
            return Err(Error::Safety(
                "live changes would be overwritten; review drift or pass --overwrite-conflicts"
                    .to_string(),
            ));
        }
        let snapshot_dir = target_runtime_path(target)
            .join("config-snapshots")
            .join(timestamp_label());
        fs::create_dir_all(&snapshot_dir)?;
        let mut metadata = BTreeMap::new();
        println!("Config apply: {}", target.name);
        for entry in drift_entries {
            if entry.state == ConfigDriftState::Unchanged {
                println!("unchanged {}", entry.relative.display());
                metadata.insert(
                    normalize_path(&entry.relative),
                    (
                        normalize_path(&entry.relative),
                        entry.source_hash.clone(),
                        entry.live_hash.clone().unwrap_or(entry.source_hash),
                    ),
                );
                continue;
            }
            let relative = entry.relative;
            let source = source_dir.join(&relative);
            let live = live_dir.join(&relative);
            if live.exists() {
                let snapshot = snapshot_dir.join(&relative);
                if let Some(parent) = snapshot.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&live, snapshot)?;
            }
            if let Some(parent) = live.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source, &live)?;
            let source_hash = sha256_of_file(&source)?;
            let live_hash = sha256_of_file(&live)?;
            metadata.insert(
                normalize_path(&relative),
                (normalize_path(&relative), source_hash, live_hash),
            );
            println!("applied {}", relative.display());
        }
        write_last_applied(target, metadata)?;
        Ok(())
    }

    fn drift_entries(&self, target: &Instance) -> Result<Vec<ConfigDrift>> {
        let source_dir = self.source_config_dir(target);
        let live_dir = target_live_path(target)?;
        let managed = list_files_recursive(&source_dir)?;
        let last_applied = read_last_applied(target)?;
        let mut entries = Vec::new();
        for relative in managed {
            let source = source_dir.join(&relative);
            let live = live_dir.join(&relative);
            let source_hash = sha256_of_file(&source)?;
            let live_hash = if live.exists() {
                Some(sha256_of_file(&live)?)
            } else {
                None
            };
            let key = normalize_path(&relative);
            let state = match (last_applied.get(&key), live_hash.as_deref()) {
                (_, None) => ConfigDriftState::MissingLive,
                (Some(last), Some(live_hash)) => {
                    let repo_changed = source_hash != last.source_sha256;
                    let live_changed = live_hash != last.live_sha256_after_apply;
                    match (repo_changed, live_changed) {
                        (false, false) => ConfigDriftState::Unchanged,
                        (true, false) => ConfigDriftState::RepoChanged,
                        (false, true) => ConfigDriftState::LiveChanged,
                        (true, true) => ConfigDriftState::Conflict,
                    }
                }
                (None, Some(live_hash)) => {
                    if live_hash == source_hash {
                        ConfigDriftState::Unchanged
                    } else {
                        ConfigDriftState::UntrackedLive
                    }
                }
            };
            entries.push(ConfigDrift {
                relative,
                source_hash,
                live_hash,
                state,
            });
        }
        Ok(entries)
    }

    pub fn import(&self, target: &Instance, relative: &Path) -> Result<()> {
        validate_relative_path(relative)?;
        let source_dir = self.source_config_dir(target);
        let live_dir = target_live_path(target)?;
        let source = source_dir.join(relative);
        let live = live_dir.join(relative);
        if !live.exists() {
            return Err(Error::InvalidArgument(format!(
                "live config does not exist: {}",
                live.display()
            )));
        }
        if let Some(parent) = source.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&live, &source)?;
        println!("imported {} into {}", live.display(), source.display());
        Ok(())
    }

    fn source_config_dir(&self, target: &Instance) -> PathBuf {
        self.repo_root
            .join("instances")
            .join(&target.name)
            .join("configs")
    }
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new()
    }
}

pub struct BackupManager {
    repo_root: PathBuf,
}

impl BackupManager {
    pub fn new() -> Self {
        Self {
            repo_root: PathBuf::from("."),
        }
    }

    pub fn preflight(&self, target: &Instance) -> Result<()> {
        let live = target_live_path(target)?;
        let runtime = target_runtime_path(target);
        println!("Backup preflight: {}", target.name);
        println!("  live tree: {}", live.display());
        println!("  live tree exists: {}", live.exists());
        println!("  runtime metadata: {}", runtime.display());
        println!("  runtime metadata exists: {}", runtime.exists());
        println!("World/state candidates:");
        for name in ["world", "world_nether", "world_the_end", "plugins"] {
            let path = live.join(name);
            println!(
                "  {} {}",
                if path.exists() { "present" } else { "missing" },
                path.display()
            );
        }
        let policy_file = self
            .repo_root
            .join("instances")
            .join(&target.name)
            .join("plugin-policy.yaml");
        if policy_file.exists() {
            println!("Policy: {}", policy_file.display());
            if file_contains_case_insensitive(&policy_file, "requires_external_db_snapshot")? {
                println!("Warning: external DB snapshot is required by plugin policy.");
            }
        } else {
            println!("Policy: missing");
        }
        println!("No backup repository was changed. DB and backup storage remain external.");
        Ok(())
    }
}

impl Default for BackupManager {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MaintenanceManager;

impl MaintenanceManager {
    pub fn new() -> Self {
        Self
    }

    pub fn restart(&self, target: &Instance, notice: Option<&str>) -> Result<()> {
        let start = Instant::now();
        if let Some(notice) = notice {
            RconCommandSender::new().send_command(target, &format!("say {notice}"))?;
        }
        runtime_lifecycle(target, "restart")?;
        let event = OperationEvent::operation(
            &target.name,
            target.runtime.as_deref().unwrap_or("systemd-java"),
            "maintenance.restart",
            true,
            start.elapsed().as_millis(),
        );
        let _ = write_operation_event(&event);
        Ok(())
    }
}

impl Default for MaintenanceManager {
    fn default() -> Self {
        Self::new()
    }
}

fn write_last_applied(
    target: &Instance,
    metadata: BTreeMap<String, (String, String, String)>,
) -> Result<()> {
    let path = target_runtime_path(target).join("last-applied.json");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut lines = Vec::new();
    lines.push("{".to_string());
    lines.push("  \"managed_files\": {".to_string());
    for (index, (target_path, (source, source_hash, live_hash))) in metadata.iter().enumerate() {
        let comma = if index + 1 == metadata.len() { "" } else { "," };
        lines.push(format!("    \"{}\": {{", json_escape(target_path)));
        lines.push(format!("      \"source\": \"{}\",", json_escape(source)));
        lines.push(format!(
            "      \"source_sha256\": \"{}\",",
            json_escape(source_hash)
        ));
        lines.push(format!(
            "      \"live_sha256_after_apply\": \"{}\",",
            json_escape(live_hash)
        ));
        lines.push(format!("      \"applied_at\": \"{}\"", timestamp_label()));
        lines.push(format!("    }}{comma}"));
    }
    lines.push("  }".to_string());
    lines.push("}".to_string());
    fs::write(path, lines.join("\n") + "\n")?;
    Ok(())
}

#[derive(Debug, Clone)]
struct LastAppliedFile {
    source_sha256: String,
    live_sha256_after_apply: String,
}

fn read_last_applied(target: &Instance) -> Result<BTreeMap<String, LastAppliedFile>> {
    let path = target_runtime_path(target).join("last-applied.json");
    let mut files = BTreeMap::new();
    if !path.exists() {
        return Ok(files);
    }
    let content = fs::read_to_string(path)?;
    let mut current_path: Option<String> = None;
    let mut current_source: Option<String> = None;
    let mut current_live: Option<String> = None;
    for line in content.lines() {
        let trimmed = line.trim().trim_end_matches(',');
        if trimmed.starts_with('"') && trimmed.ends_with('{') {
            if let Some(path) = current_path.take() {
                if let (Some(source_sha256), Some(live_sha256_after_apply)) =
                    (current_source.take(), current_live.take())
                {
                    files.insert(
                        path,
                        LastAppliedFile {
                            source_sha256,
                            live_sha256_after_apply,
                        },
                    );
                }
            }
            current_path = trimmed
                .split_once(':')
                .map(|(path, _)| unquote(path.trim()).to_string());
        } else if let Some((key, value)) = trimmed.split_once(':') {
            let key = unquote(key.trim());
            let value = unquote(value.trim());
            match key {
                "source_sha256" => current_source = Some(value.to_string()),
                "live_sha256_after_apply" => current_live = Some(value.to_string()),
                _ => {}
            }
        }
    }
    if let Some(path) = current_path.take() {
        if let (Some(source_sha256), Some(live_sha256_after_apply)) =
            (current_source.take(), current_live.take())
        {
            files.insert(
                path,
                LastAppliedFile {
                    source_sha256,
                    live_sha256_after_apply,
                },
            );
        }
    }
    Ok(files)
}

fn list_jar_files(path: &Path) -> Result<BTreeSet<String>> {
    let mut jars = BTreeSet::new();
    if !path.exists() {
        return Ok(jars);
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.to_ascii_lowercase().ends_with(".jar") {
            jars.insert(name);
        }
    }
    Ok(jars)
}

fn list_files_recursive(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !root.exists() {
        return Ok(files);
    }
    collect_files(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_files(root: &Path, current: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_files(root, &path, files)?;
        } else if file_type.is_file() {
            let relative = path.strip_prefix(root).map_err(|error| {
                Error::Runtime(format!(
                    "failed to normalize path {}: {error}",
                    path.display()
                ))
            })?;
            files.push(relative.to_path_buf());
        }
    }
    Ok(())
}

fn target_live_path(target: &Instance) -> Result<PathBuf> {
    target
        .paths
        .live
        .clone()
        .or_else(|| target.paths.root.as_ref().map(|root| root.join("data")))
        .ok_or_else(|| Error::Inventory(format!("missing live path for {}", target.name)))
}

fn target_runtime_path(target: &Instance) -> PathBuf {
    target
        .paths
        .root
        .clone()
        .unwrap_or_else(|| PathBuf::from("/srv/kitsunebi/instances").join(&target.name))
        .join("runtime")
}

fn sha256_of_file(path: &Path) -> Result<String> {
    let output = Command::new("sha256sum").arg(path).output()?;
    if !output.status.success() {
        return Err(Error::Runtime(format!(
            "sha256sum failed for {}",
            path.display()
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let hash = text
        .split_whitespace()
        .next()
        .ok_or_else(|| Error::Runtime("sha256sum returned no output".to_string()))?;
    Ok(format!("sha256:{hash}"))
}

fn validate_relative_path(path: &Path) -> Result<()> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| component.as_os_str() == OsStr::new(".."))
    {
        return Err(Error::InvalidArgument(format!(
            "path must be relative and must not contain '..': {}",
            path.display()
        )));
    }
    Ok(())
}

fn normalize_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn timestamp_label() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    seconds.to_string()
}

fn print_name_set(label: &str, values: &BTreeSet<String>) {
    println!("{label}:");
    if values.is_empty() {
        println!("  none");
    } else {
        for value in values {
            println!("  {value}");
        }
    }
}

fn jar_stem(jar: &str) -> String {
    Path::new(jar)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_else(|| jar.to_ascii_lowercase())
}

fn yaml_key(key: &str) -> String {
    if key
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '/'))
    {
        key.to_string()
    } else {
        format!("\"{}\"", json_escape(key))
    }
}

fn file_contains_case_insensitive(path: &Path, needle: &str) -> Result<bool> {
    let content = fs::read_to_string(path)?;
    Ok(content
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase()))
}

fn run_status(command: &mut Command) -> Result<()> {
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Runtime(format!(
            "command failed with status {status}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_inventory_subset() {
        let source = r#"
nodes:
  - name: kng01-game-01
    address: 10.10.30.20
    default_runtime: systemd-java

instances:
  - name: backend-vanilla-1
    role: backend
    node: kng01-game-01
    runtime: systemd-java
    paths:
      root: /srv/kitsunebi/instances/backend-vanilla-1
      live: /srv/kitsunebi/instances/backend-vanilla-1/data
    rcon:
      enabled: true
      host: 127.0.0.1
      port: 25576
      secret_ref: backend-vanilla-1/rcon
    logs:
      journald_unit: kitsunebi@backend-vanilla-1.service
"#;
        let inventory = parse_inventory(source).expect("inventory parses");
        assert_eq!(inventory.nodes[0].name, "kng01-game-01");
        let instance = inventory
            .resolve("backend-vanilla-1")
            .expect("target exists");
        assert_eq!(instance.rcon.port, Some(25576));
        assert_eq!(
            instance.paths.live.as_deref(),
            Some(Path::new("/srv/kitsunebi/instances/backend-vanilla-1/data"))
        );
    }

    #[test]
    fn masks_sensitive_command_parts() {
        let masked = mask_command_preview("say hi token=abc password=secret key=value");
        assert_eq!(masked, "say hi token=*** password=*** key=***");
    }

    #[test]
    fn rejects_absolute_import_path() {
        assert!(validate_relative_path(Path::new("/etc/passwd")).is_err());
        assert!(validate_relative_path(Path::new("../server.properties")).is_err());
        assert!(validate_relative_path(Path::new("plugins/LuckPerms/config.yml")).is_ok());
    }

    #[test]
    fn config_apply_writes_metadata_and_blocks_untracked_live_drift() {
        let root = temp_test_dir("config-apply");
        let repo = root.join("repo");
        let instance_root = root.join("instance");
        let live = instance_root.join("data");
        fs::create_dir_all(repo.join("instances/test-1/configs/plugins/LuckPerms"))
            .expect("create repo config");
        fs::create_dir_all(live.join("plugins/LuckPerms")).expect("create live config");
        fs::write(
            repo.join("instances/test-1/configs/plugins/LuckPerms/config.yml"),
            "storage-method: h2\n",
        )
        .expect("write repo config");
        fs::write(
            live.join("plugins/LuckPerms/config.yml"),
            "storage-method: mysql\n",
        )
        .expect("write live config");

        let manager = ConfigManager { repo_root: repo };
        let instance = test_instance("test-1", &instance_root, &live);
        assert!(manager.apply(&instance, false).is_err());
        manager
            .apply(&instance, true)
            .expect("explicit overwrite applies");
        let metadata = instance_root.join("runtime/last-applied.json");
        assert!(metadata.exists());
        assert_eq!(
            fs::read_to_string(live.join("plugins/LuckPerms/config.yml")).expect("read live"),
            "storage-method: h2\n"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn plugin_lock_records_manual_artifacts() {
        let root = temp_test_dir("plugin-lock");
        let manual_dir = root.join("plugins/manual/test-1");
        fs::create_dir_all(&manual_dir).expect("create manual dir");
        fs::write(manual_dir.join("LuckPerms.jar"), "fake jar").expect("write jar");
        let manager = PluginManager {
            repo_root: root.clone(),
        };
        let inventory = Inventory {
            nodes: Vec::new(),
            instances: vec![test_instance(
                "test-1",
                &root.join("instance"),
                &root.join("live"),
            )],
        };
        manager.lock(&inventory).expect("lock writes");
        let lock = fs::read_to_string(root.join("plugins/plugins.lock")).expect("read lock");
        assert!(lock.contains("test-1/luckperms"));
        assert!(lock.contains("LuckPerms.jar"));
        assert!(lock.contains("sha256:"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn plugin_three_way_diff_requires_all_three_files() {
        let root = temp_test_dir("three-way");
        let repo = root.join("repo");
        let instance_root = root.join("instance");
        let live = instance_root.join("data");
        fs::create_dir_all(repo.join("instances/test-1/configs/plugins/LuckPerms"))
            .expect("create repo config");
        fs::create_dir_all(live.join("plugins/LuckPerms")).expect("create live config");
        let relative = Path::new("plugins/LuckPerms/config.yml");
        fs::write(
            repo.join("instances/test-1/configs").join(relative),
            "storage-method: h2\n",
        )
        .expect("write repo");
        fs::write(live.join(relative), "storage-method: mysql\n").expect("write live");
        let migrated = root.join("migrated.yml");
        fs::write(&migrated, "storage-method: mysql\n").expect("write migrated");
        let manager = PluginManager {
            repo_root: repo.clone(),
        };
        let instance = test_instance("test-1", &instance_root, &live);
        assert!(
            manager
                .three_way_diff(&instance, relative, &migrated)
                .is_ok()
        );
        assert!(
            manager
                .three_way_diff(&instance, Path::new("../bad.yml"), &migrated)
                .is_err()
        );
        let _ = fs::remove_dir_all(root);
    }

    fn test_instance(name: &str, root: &Path, live: &Path) -> Instance {
        Instance {
            name: name.to_string(),
            role: Some("backend".to_string()),
            node: Some("local".to_string()),
            runtime: Some("systemd-java".to_string()),
            paths: InstancePaths {
                root: Some(root.to_path_buf()),
                live: Some(live.to_path_buf()),
            },
            rcon: RconConfig::default(),
            logs: LogConfig::default(),
        }
    }

    fn temp_test_dir(name: &str) -> PathBuf {
        let path = env::temp_dir().join(format!(
            "kitsunebi-{name}-{}-{}",
            std::process::id(),
            timestamp_label()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp test dir");
        path
    }
}
