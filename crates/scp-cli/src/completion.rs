use scp_core::error::CoreError;

const COMMANDS_TOP: &str =
    "schema capabilities status server config settings openvpn boundary caddy scp mcp completion";
const GLOBAL_FLAGS: &str =
    "--help --version --config-dir --profile --json --interactive --non-interactive";
const SUBCOMMAND_FLAGS: &str = "--no-profile --stdio --foreground --recursive --overwrite --command --function-id --limit --after --server-id --filter --folder --local --remote --shell --file --stdin --preview --expected-revision --secret-file --secret-stdin --key --repo-root --output --input --passphrase-file --passphrase-stdin --apply --no-shared --no-servers --no-local --no-secrets --use-source-paths --remap";

/// Generates a shell completion script without opening a profile or unlocking a keyring.
pub fn generate_completion(shell: &str) -> Result<String, CoreError> {
    match shell.trim() {
        "bash" => Ok(bash_completion()),
        "zsh" => Ok(zsh_completion()),
        "fish" => Ok(fish_completion()),
        "powershell" => Ok(powershell_completion()),
        other => Err(CoreError::invalid_arguments(
            "unsupported_shell",
            format!("Unsupported shell '{other}'; expected one of: bash, zsh, fish, powershell"),
        )),
    }
}

fn bash_completion() -> String {
    format!(
        r#"_scpctl_completions() {{
    local cur prev words cword
    cur="${{COMP_WORDS[COMP_CWORD]}}"
    prev=""
    if (( COMP_CWORD > 0 )); then
        prev="${{COMP_WORDS[COMP_CWORD-1]}}"
    fi
    if [[ "$prev" == "completion" || "$prev" == "--shell" ]]; then
        COMPREPLY=( $(compgen -W "bash zsh fish powershell" -- "$cur") )
        return 0
    fi
    if [[ "$cur" == -* ]]; then
        COMPREPLY=( $(compgen -W "{GLOBAL_FLAGS} {SUBCOMMAND_FLAGS}" -- "$cur") )
        return 0
    fi
    case "${{COMP_WORDS[1]:-}}" in
        server) COMPREPLY=( $(compgen -W "list get upsert delete tunnel ssh rdp credentials" -- "$cur") ) ;;
        config) COMPREPLY=( $(compgen -W "paths save patch secret sync" -- "$cur") ) ;;
        settings) COMPREPLY=( $(compgen -W "show patch export import" -- "$cur") ) ;;
        openvpn) COMPREPLY=( $(compgen -W "detect start" -- "$cur") ) ;;
        boundary) COMPREPLY=( $(compgen -W "targets auth" -- "$cur") ) ;;
        caddy) COMPREPLY=( $(compgen -W "start" -- "$cur") ) ;;
        scp) COMPREPLY=( $(compgen -W "upload download" -- "$cur") ) ;;
        mcp) COMPREPLY=( $(compgen -W "serve" -- "$cur") ) ;;
        completion) COMPREPLY=( $(compgen -W "bash zsh fish powershell" -- "$cur") ) ;;
        *) COMPREPLY=( $(compgen -W "{COMMANDS_TOP}" -- "$cur") ) ;;
    esac
}}
complete -F _scpctl_completions scpctl
"#
    )
}

fn zsh_completion() -> String {
    format!(
        r#"#compdef scpctl
_scpctl() {{
    local -a top flags
    top=(schema capabilities status server config settings openvpn boundary caddy scp mcp completion)
    flags=({GLOBAL_FLAGS} {SUBCOMMAND_FLAGS})
    if [[ $words[CURRENT] == -* ]]; then
        _describe 'flag' flags
        return
    fi
    case $words[2] in
        server) _values 'subcommand' list get upsert delete tunnel ssh rdp credentials ;;
        config) _values 'subcommand' paths save patch secret sync ;;
        settings) _values 'subcommand' show patch export import ;;
        openvpn) _values 'subcommand' detect start ;;
        boundary) _values 'subcommand' targets auth ;;
        caddy) _values 'subcommand' start ;;
        scp) _values 'subcommand' upload download ;;
        mcp) _values 'subcommand' serve ;;
        completion) _values 'shell' bash zsh fish powershell ;;
        *) _describe 'command' top ;;
    esac
}}
compdef _scpctl scpctl
"#
    )
}

fn fish_completion() -> String {
    let mut out = String::new();
    for top in COMMANDS_TOP.split_whitespace() {
        out.push_str(&format!(
            "complete -c scpctl -f -n '__fish_use_subcommand' -a '{top}'\n"
        ));
    }
    for (parent, children) in [
        (
            "server",
            "list get upsert delete tunnel ssh rdp credentials",
        ),
        ("config", "paths save patch secret sync"),
        ("settings", "show patch export import"),
        ("openvpn", "detect start"),
        ("boundary", "targets auth"),
        ("caddy", "start"),
        ("scp", "upload download"),
        ("mcp", "serve"),
        ("completion", "bash zsh fish powershell"),
    ] {
        for child in children.split_whitespace() {
            out.push_str(&format!(
                "complete -c scpctl -f -n '__fish_seen_subcommand_from {parent}' -a '{child}'\n"
            ));
        }
    }
    for flag in GLOBAL_FLAGS
        .split_whitespace()
        .chain(SUBCOMMAND_FLAGS.split_whitespace())
    {
        let name = flag.trim_start_matches('-');
        out.push_str(&format!("complete -c scpctl -f -l '{name}'\n"));
    }
    out
}

fn powershell_completion() -> String {
    format!(
        r#"Register-ArgumentCompleter -Native -CommandName scpctl -ScriptBlock {{
    param($wordToComplete, $commandAst, $cursorPosition)
    $tokens = @('{top}' -split ' ') + @('{global}' -split ' ') + @('{sub}' -split ' ')
    $shells = @('bash', 'zsh', 'fish', 'powershell')
    $all = $tokens + $shells + @('list','get','upsert','delete','tunnel','ssh','rdp','credentials','set','clear','save','patch','secret','sync','push','show','export','import','detect','start','targets','search','auth','upload','download','serve','paths')
    $all | Where-Object {{ $_ -like "$wordToComplete*" }} | ForEach-Object {{
        [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)
    }}
}}
"#,
        top = COMMANDS_TOP,
        global = GLOBAL_FLAGS,
        sub = SUBCOMMAND_FLAGS
    )
}
