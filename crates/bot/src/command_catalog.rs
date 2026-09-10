//! Single source of truth for command discovery.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandRole {
    User,
    Admin,
}

#[derive(Debug, Clone, Copy)]
pub struct CommandDescriptor {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub role: CommandRole,
    pub summary: &'static str,
    pub usage: &'static str,
}

pub const COMMANDS: &[CommandDescriptor] = &[
    CommandDescriptor {
        name: "start",
        aliases: &[],
        role: CommandRole::User,
        summary: "Show the quick start guide",
        usage: "/start",
    },
    CommandDescriptor {
        name: "alac",
        aliases: &["rip", "batch", "dl", "download"],
        role: CommandRole::User,
        summary: "Download a track, album, playlist, or artist",
        usage: "/alac <link | id> [--zip]",
    },
    CommandDescriptor {
        name: "zip",
        aliases: &[],
        role: CommandRole::User,
        summary: "Package a multi-track album as a ZIP",
        usage: "/zip <album link | id>",
    },
    CommandDescriptor {
        name: "search",
        aliases: &[],
        role: CommandRole::User,
        summary: "Search cached tracks and the Apple Music catalog",
        usage: "/search <query>",
    },
    CommandDescriptor {
        name: "status",
        aliases: &[],
        role: CommandRole::User,
        summary: "View active downloads",
        usage: "/status",
    },
    CommandDescriptor {
        name: "cancel",
        aliases: &[],
        role: CommandRole::User,
        summary: "Cancel your active download",
        usage: "/cancel",
    },
    CommandDescriptor {
        name: "info",
        aliases: &[],
        role: CommandRole::User,
        summary: "Inspect metadata and cache status",
        usage: "/info <link | id>",
    },
    CommandDescriptor {
        name: "spec",
        aliases: &["spectogram", "spectrogram", "spek"],
        role: CommandRole::User,
        summary: "Generate a spectrogram from replied audio",
        usage: "/spec",
    },
    CommandDescriptor {
        name: "report",
        aliases: &["issue"],
        role: CommandRole::User,
        summary: "Report a problem with a track",
        usage: "/report",
    },
    CommandDescriptor {
        name: "help",
        aliases: &[],
        role: CommandRole::User,
        summary: "Show this guide",
        usage: "/help",
    },
    CommandDescriptor {
        name: "settings",
        aliases: &[],
        role: CommandRole::Admin,
        summary: "Change bot operating settings",
        usage: "/settings",
    },
    CommandDescriptor {
        name: "dumpnew",
        aliases: &["autodump"],
        role: CommandRole::Admin,
        summary: "Seed new Apple Music releases",
        usage: "/dumpnew <days>",
    },
    CommandDescriptor {
        name: "cache",
        aliases: &["dump"],
        role: CommandRole::Admin,
        summary: "Seed tracks into the dump channel",
        usage: "/cache <link>",
    },
    CommandDescriptor {
        name: "random",
        aliases: &[],
        role: CommandRole::Admin,
        summary: "Discover and seed a random album",
        usage: "/random",
    },
    CommandDescriptor {
        name: "delete",
        aliases: &[],
        role: CommandRole::Admin,
        summary: "Delete a cached track",
        usage: "/delete <track_id>",
    },
    CommandDescriptor {
        name: "auth",
        aliases: &[],
        role: CommandRole::Admin,
        summary: "Authorize a user or group",
        usage: "/auth <user_id | reply>",
    },
    CommandDescriptor {
        name: "revoke",
        aliases: &[],
        role: CommandRole::Admin,
        summary: "Revoke authorization",
        usage: "/revoke <user_id | reply>",
    },
    CommandDescriptor {
        name: "authlist",
        aliases: &[],
        role: CommandRole::Admin,
        summary: "List authorized users",
        usage: "/authlist",
    },
    CommandDescriptor {
        name: "stats",
        aliases: &[],
        role: CommandRole::Admin,
        summary: "View download and cache statistics",
        usage: "/stats",
    },
    CommandDescriptor {
        name: "clean",
        aliases: &[],
        role: CommandRole::Admin,
        summary: "Remove leftover temporary files",
        usage: "/clean",
    },
    CommandDescriptor {
        name: "ping",
        aliases: &["health"],
        role: CommandRole::Admin,
        summary: "Check Telegram, database, and mirror health",
        usage: "/ping",
    },
    CommandDescriptor {
        name: "queue",
        aliases: &[],
        role: CommandRole::Admin,
        summary: "View active and pending downloads",
        usage: "/queue",
    },
    CommandDescriptor {
        name: "index",
        aliases: &[],
        role: CommandRole::Admin,
        summary: "Reconcile the dump channel with the database",
        usage: "/index",
    },
    CommandDescriptor {
        name: "export",
        aliases: &[],
        role: CommandRole::Admin,
        summary: "Export a database archive",
        usage: "/export",
    },
    CommandDescriptor {
        name: "import",
        aliases: &[],
        role: CommandRole::Admin,
        summary: "Restore a database archive",
        usage: "/import",
    },
];

pub fn render_help(is_admin: bool) -> String {
    let mut out = String::from("<b>ALAC Bot</b>\n\n<b>Start here</b>\n");
    for command in COMMANDS
        .iter()
        .filter(|command| command.role == CommandRole::User)
    {
        out.push_str(&format!(
            "• {} — {}\n",
            display_usage(command.usage),
            command.summary
        ));
    }
    out.push_str("\n<i>Audio requested in a group is delivered to your private chat.</i>");
    if is_admin {
        out.push_str("\n\n<b>Admin commands</b>\n");
        for command in COMMANDS
            .iter()
            .filter(|command| command.role == CommandRole::Admin)
        {
            out.push_str(&format!(
                "• {} — {}\n",
                display_usage(command.usage),
                command.summary
            ));
        }
    }
    out
}

fn display_usage(usage: &str) -> String {
    usage.replace('<', "[").replace('>', "]")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_is_unique_and_role_aware() {
        let mut names = COMMANDS
            .iter()
            .map(|command| command.name)
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), COMMANDS.len());
        assert!(!render_help(false).contains("/settings"));
        assert!(render_help(true).contains("/settings"));
        assert!(render_help(false).contains("cached tracks and the Apple Music catalog"));
        assert!(render_help(false).contains("/alac [link | id]"));
        assert!(!render_help(false).contains("<link | id>"));
    }
}
