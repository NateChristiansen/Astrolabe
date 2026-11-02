//! The launch contract: a repo, a provider and the dials in; a protected `Launch` out. Here
//! the registry, the profile, `Layer::protected` and the worktree code meet.
use crate::acp::Launch;
use crate::driver::{claude, codex};
use crate::error::{Error, Result};
use crate::hook::comparable;
use crate::layer::{Layer, RepoInstructions, RepoSupplies, Target, TargetKind, compose};
use crate::registry::{self, Registry};
use crate::trust::Homes;
use crate::worktree::{self, Progress, Worktree};
use std::path::{Path, PathBuf};

const BASE: &str = "This session runs under Astrolabe. The repo's own workflow comes first.";
/// Design 4.2's one-line reading. It goes to the session; no control sets it yet.
pub const DEFAULT_DIALS: &str = "change · medium · edit · autonomous · one piece";

pub type Say<'a> = &'a mut dyn FnMut(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub const ALL: [Self; 2] = [Self::Claude, Self::Codex];

    pub fn slug(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

pub struct Machine {
    pub registry: Registry,
    pub data_dir: PathBuf,
    pub program_dir: PathBuf,
    pub adapters: PathBuf,
    pub homes: Homes,
}

impl Machine {
    pub fn from_env() -> Result<Self> {
        let set = std::env::var_os(crate::layer::DATA_DIR_VAR).map(PathBuf::from);
        let default = || crate::default_db_path()?.parent().map(Path::to_path_buf);
        let none = || Error::Setup("no app-data folder found".to_owned());
        Self::at(set.or_else(default).ok_or_else(none)?)
    }

    pub fn at(data_dir: PathBuf) -> Result<Self> {
        let adapters = std::env::var_os("ASTROLABE_ADAPTERS").map(PathBuf::from);
        let exe = std::env::current_exe()?;
        Ok(Self {
            registry: Registry::open(&data_dir.join("astrolabe.db"))?,
            adapters: adapters.unwrap_or_else(|| data_dir.join("adapters")),
            program_dir: exe.parent().unwrap_or(Path::new(".")).to_path_buf(),
            homes: Homes::from_env()?,
            data_dir,
        })
    }

    pub fn db(&self) -> PathBuf {
        self.data_dir.join("astrolabe.db")
    }
}

pub struct Request<'a> {
    pub repo: &'a str,
    pub provider: Provider,
    /// Names the worktree and branch on a product repo.
    pub session: &'a str,
    pub dials: &'a str,
    pub client: &'a Path,
}

#[derive(Debug)]
pub struct Prepared {
    pub launch: Launch,
    pub cwd: PathBuf,
    /// None on the agent repo: trust lines name its own folder, so it runs in place.
    pub worktree: Option<Worktree>,
}

/// The dials, then the repo's private note from the agent repo, which only fills gaps.
pub fn instructions(agent_repo: Option<&Path>, name: &str, dials: &str) -> String {
    let file = |repo: &Path| repo.join("knowledge/repos").join(format!("{name}.md"));
    let note = agent_repo.and_then(|repo| std::fs::read_to_string(file(repo)).ok());
    let reading = format!("{BASE}\n\nDials: {dials}.");
    match note {
        Some(text) => format!("{reading}\n\nRepo profile, where the repo is silent:\n\n{text}"),
        None => reading,
    }
}

fn make_worktree(repo: &Path, name: &str, say: Say) -> Result<Worktree> {
    let path = worktree::path_for(repo, name)?;
    crate::doctor::refuse(&path)?;
    let mut creating = worktree::create(repo, name)?;
    loop {
        match creating.poll() {
            Some(Progress::Creating { elapsed }) => {
                say(format!("creating worktree, {} s", elapsed.as_secs()));
            }
            Some(Progress::Ready(made)) => return Ok(made),
            Some(Progress::Failed(why)) => return Err(Error::Setup(why)),
            None => return Err(Error::Setup("worktree creation stopped".to_owned())),
        }
    }
}

/// An agent repo holding the real layer supplies it all, and Codex's hook check refuses it
/// untrusted. Anything else gets the whole layer, and a repo with its own
/// `developer_instructions` reads Unknown, as Codex's setting would replace them.
pub fn target_for(layer: &Layer, repo: &Path, agent: bool) -> Target {
    let states = crate::init::layer_state(repo);
    let real = states.iter().all(|(_, state)| *state == Some(true));
    if agent && real {
        let supplies = RepoSupplies {
            instructions: RepoInstructions::Text(layer.instructions.clone()),
            hooks: layer.hooks.clone(),
            mcp: layer.mcp.clone(),
            denies: layer.denies.clone(),
            memory_off: true,
        };
        let kind = TargetKind::Agent;
        return Target { kind, supplies };
    }
    let own = std::fs::read_to_string(repo.join(".codex/config.toml"));
    let has_own = own.is_ok_and(|text| text.contains("developer_instructions"));
    let mut supplies = RepoSupplies::default();
    if !has_own {
        supplies.instructions = RepoInstructions::None;
    }
    let kind = TargetKind::Product;
    Target { kind, supplies }
}

fn build(
    machine: &Machine,
    request: &Request,
    target: &Target,
    layer: &Layer,
    cwd: &Path,
) -> Result<Launch> {
    let mut delivery = compose(target, layer)?;
    own_tool_server(&mut delivery, &machine.program_dir);
    let (adapters, client) = (&machine.adapters, Some(request.client));
    let (mut launch, at) = match request.provider {
        Provider::Claude => {
            let folder = machine.data_dir.join("instructions");
            let delivered = claude::deliver(&delivery, &folder.join(request.session));
            delivered.write_instructions()?;
            (delivered.launch(adapters, client)?, claude::RECEIPT_AT)
        }
        Provider::Codex => {
            let mut keys = Vec::new();
            if !delivery.hooks.is_empty() {
                keys = codex::discover_keys(request.client, cwd, &delivery.hooks)?;
            }
            let delivered = codex::deliver(&delivery, &keys)?;
            (delivered.launch(adapters, client)?, codex::RECEIPT_AT)
        }
    };
    launch.folder_check = Some(crate::doctor::folder_check);
    let (program, data) = (&machine.program_dir, &machine.data_dir);
    let protected = launch.protect(at, program, data, std::env::var_os("PATH"));
    protected.map_err(|error| Error::Setup(error.to_string()))?;
    Ok(launch)
}

/// Syncs, makes the worktree and composes, saying so. `AcpClient::launch` takes the result.
pub fn prepare(machine: &Machine, request: &Request, say: Say) -> Result<Prepared> {
    let registry = &machine.registry;
    if !worktree::valid_name(request.session) {
        let name = request.session;
        return Err(Error::Setup(format!("session name {name:?} is not safe")));
    }
    let slug = request.provider.slug();
    if !registry.provider_on(slug)? {
        return Err(Error::Setup(format!("{slug} is off on this machine")));
    }
    let repo = registry.checkout(request.repo)?;
    let holder = registry.agent_repo()?;
    let is_repo = |a: &Path| comparable(a) == comparable(&repo);
    let agent = holder.as_deref().is_some_and(is_repo);
    match &holder {
        Some(repo) => drop(crate::voice::read(repo).map_err(Error::Setup)?),
        None => say(crate::voice::NO_AGENT.to_owned()),
    }
    say("syncing".to_owned());
    if let Err(why) = registry::sync(&repo) {
        say(format!("Not synced, so using your local copy.\n{why}"));
    }
    let mut made = None;
    if !agent {
        made = Some(make_worktree(&repo, request.session, say)?);
    }
    let cwd = made.as_ref().map_or(&repo, |made| &made.path).clone();
    let note = holder.as_deref().filter(|_| !agent);
    let layer = Layer::protected(&instructions(note, request.repo, request.dials));
    let target = target_for(&layer, &cwd, agent);
    let built = build(machine, request, &target, &layer, &cwd);
    if let (Err(_), Some(made)) = (&built, &made) {
        let _ = worktree::close(made);
    }
    let (launch, worktree) = (built?, made);
    Ok(Prepared {
        launch,
        cwd,
        worktree,
    })
}

/// The core's own binary for the tool server of a session it launches. After `compose`, which
/// compares a repo's copy by the bare name; hooks stay bare, as hook trust hashes their text.
pub fn own_tool_server(delivery: &mut crate::layer::Delivery, program_dir: &Path) {
    use crate::layer::PROGRAM;
    let own = program_dir.join(format!("{PROGRAM}{}", std::env::consts::EXE_SUFFIX));
    for server in delivery.mcp.iter_mut().filter(|s| s.command == PROGRAM) {
        server.command = own.to_string_lossy().into_owned();
    }
}
