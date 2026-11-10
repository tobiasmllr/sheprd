---
title: Issues and pull requests
blurb: From an issue to an agent in one click.
status: shipped
---

# Issues and pull requests

The orchestrator lists the issues, pull requests, and Beads tasks of every repository your workspaces hold, and starts an agent on any of them in a new worktree. This page covers the sections it adds to the activity bar, its sources, dispatching agents, and review profiles.

## Where it lives

The activity bar at the window's left edge has **Pull requests**, **Issues**, and **Tasks** sections. Each takes the window's body, keeping the title bar and status bar. Together they cover every repository held by the workspaces of every connected local or SSH host, up to 32 repositories. Worktrees of one repository count once.

Other ways in:

- The **Issues** row of a group's **…** menu or a workspace's popover, and the **Show Issues** command, open Issues narrowed to that workspace's project.
- `sheprd orchestrator` does the same from a pane, and `sheprd ui panel issues` (or `pull-requests`, `tasks`) shows a section.
- `sheprd orchestrator items|issues|prs|tasks` prints what was last synced, without the app running.

A section's icon shows its count in the accent color until you open it, then in gray until something changes again. Under the search, pills pick backends (GitHub, GitLab, Forgejo, Beads) and projects, each with its count; with none picked, everything is listed. The window button in the header moves the view into a window of its own.

A click selects a row. Enter or a double-click opens its page in a pane beside the list; drag the pane's left edge to resize it, or press × to close it.

## Sources

| Source | What is listed | Sign-in |
| --- | --- | --- |
| GitHub | Open issues and pull requests | The GitHub account used for pull requests on that host |
| GitLab | Open issues, on gitlab.com or a self-managed instance | A token or connected account, see below |
| Forgejo | Open issues and pull requests, on Codeberg or a self-hosted instance | The Forgejo CLI's sign-in, see below |
| [Beads](https://github.com/steveyegge/beads) | Issues and tasks, when a main checkout has a `.beads` folder | None: read with `bd` |

Folders outside Git are skipped unless they have a `.beads` folder; **Refresh** checks them again. A repository without a GitHub remote has no GitHub source. GitHub Enterprise is not supported yet.

Sources sync every minute while the Pull requests or Issues section is shown, and every ten minutes otherwise. What the last sync found is kept per repository, so after a restart the lists show at once and sync after. While an SSH host cannot be reached, its projects keep what they last listed, with the error above them.

### Signing in to GitLab

The account button at the top right of the title bar switches between GitHub and GitLab. The GitLab side lists each instance, where its token comes from, and whom it signs in as. **Connect** signs in through GitLab in the browser with a one-time code (gitlab.com out of the box; a self-managed instance once `[gitlab] applications` names its OAuth application). You can also paste a personal access token with the `read_api` scope, which is checked before it is saved in your credential store.

Sheprd looks for a token in this order:

1. `GITLAB_TOKEN` or `GITLAB_ACCESS_TOKEN` set by the checkout's `.envrc`, loaded through direnv, for a checkout on this computer whose `.envrc` you allowed.
2. The same variables from your login shell, only for the instance `GITLAB_HOST` names (gitlab.com by default).
3. The account connected, or the token pasted, in the account menu.
4. The GitLab CLI's sign-in (`glab auth login`).

Public projects need none. A token is sent only over HTTPS. When GitLab needs a token, the account button shows a dot and the source shows "sign in", which opens that page.

A self-managed GitLab with a separate SSH host is read from the instance whose `ssh_host` the GitLab CLI's config names:

```sh
glab config set ssh_host gitlab-ssh.example.com --host gitlab.example.com
```

### Signing in to Forgejo

Forgejo is read with the [Forgejo CLI](https://codeberg.org/forgejo-contrib/forgejo-cli)'s own sign-in for the instance (`fj auth login` or `fj auth add-token`); Sheprd keeps no Forgejo token of its own. Public repositories need none. A token is sent only to the instance fj keeps it for, and only over HTTPS.

A self-hosted instance's name does not say that Forgejo serves it, so a remote is read as Forgejo when its host is codeberg.org, has "forgejo" in its name, or is one fj is signed in to. When an instance's repositories are cloned over SSH from another host or port, fj notes that host as the instance's alias when you sign in, and such a remote is read from the instance.

Forgejo pull requests are listed only: their status, checks, and merge pages are GitHub's.

## Issues

**Issues** lists open GitHub, GitLab, or Forgejo issues and beads together, grouped by priority. A bead's children sit under it. **Filters** narrows to items with a run, blocked ones, one source, or one label; five sorts are offered. Search matches every word you type across title, ID, author, labels, state, and body, case-insensitively unless a word has a capital.

An issue's page shows its description, its runs, and its details. **Start agent** on a row opens the dispatch dialog.

## Pull requests

**Pull requests** is a table grouped by what each waits on:

- a review asked of you,
- your own to fix (changes requested or failing checks),
- ready to merge,
- the rest,
- recently closed or merged.

Each row shows its branch, author, review state, checks as colored pills, diff size, its run, and age. A pull request's page has its conversation as on GitHub, with reviews holding their inline threads and resolved ones folded, and you can comment there. It also shows checks, review decision, and merge state. **Merge** offers the methods the repository allows and names the head commit shown, so GitHub refuses it if the branch moved.

## Dispatching an agent

**Dispatch agent** opens a dialog with:

- a prompt,
- an agent installed on the destination host,
- an optional model (passed as `--model`),
- a branch, named `agent/<number>-<slug>-<hash>` and editable,
- extra instructions.

A new worktree is created from the main checkout's `HEAD` and the agent starts in it with the prompt. Its tab and pane are named with the issue or PR number and title. With more than one host connected, the dialog ranks hosts the way [Smart Dispatch](teleport.md#smart-dispatch) does; pick another host and the repository is set up there. Dispatching to another host needs a macOS or Linux client.

On this computer, when [**Settings > Tabs > New terminals use Herdr 2**](terminals.md) is on, Git creates the worktree in the folder Herdr 1 would use, and the agent is typed into a Herdr 2 shell there with its model, then given the prompt once it is ready. The checkout gets no workspace until you open the run, which keeps one showing the agent's terminal. SSH hosts and other hosts still dispatch through Herdr 1.

A pull request's **Review with agent** does the same on an `agent/review-…` branch with a read-only prompt: the agent verifies the pull request, reads its diff, and reports findings without writing to GitHub.

### Runs

Runs have no list of their own. The Overview section shows agents and failed runs that need you, and an item's **Agent** page shows its runs, with their status from each host's live Herdr view (a Herdr 2 run shows the status it last recorded). A run's card sends the agent messages, stops it, or removes its worktree; each of those asks first. Leaving the section leaves a started dispatch running.

The hash at the end of a branch name belongs to the item, so work is found without the database, such as on another machine: a workspace on that branch, or an `agent/…` branch no workspace shows, appears as the item's run. These runs open their workspace but have no controls.

## Review profiles

The dialog uses prompt profiles from `~/.config/agent-launcher/agents/<name>/prompt.md`, or the built-in prompt when none is chosen. For a review, the read-only prompt is kept and the chosen profile is added after it.

A profile whose first line is `review: replace` is sent instead of the built-in review prompt, followed by any extra instructions, and the dialog marks it "replaces built-in". The line itself is never sent. Such a profile can use these variables:

```text
{{ pr_title }}  {{ pr_link }}  {{ pr_body }}  {{ pr_number }}
{{ pr_head_ref }}  {{ pr_head_sha }}  {{ pr_head_repository }}
{{ pr_base_ref }}  {{ pr_base_sha }}
```

> **Note.** A replacing profile drops the built-in read-only safeguards and the steps that verify the pull request, so it has to state its own. A head ref or repository name with characters beyond letters, digits, `-`, `_`, `/`, and `.` is withheld; prefer the SHAs in commands.

## Tasks

**Tasks** lists Beads tasks alone, across every project with a `.beads` folder, and shows on the bar only while one is open. Each row has its project, status, assignee, priority group, and a lock while it is blocked. **Filters** narrows by status (also how closed tasks are listed), priority, and assignee.

A task's page shows who holds it, its acceptance criteria, and the tasks it is tied to: parent, blockers, what it blocks, and children. **Open project** goes to the workspace holding it. Beads stays the source of truth: tasks are read with `bd`, and creating or editing tasks and comments is still done with `bd`.

**Start agent** on a task first claims it with `bd update --claim` on its own host, so a task someone else holds starts nothing and the banner says who holds it. The agent gets the task's ID, description, and acceptance criteria, with instructions to keep its claim alive and close the task only with a summary and validation evidence. A launch that fails gives back the claim it took, and nothing marks a task finished for the agent. Agents start on the database's own host.

## Related

- [Work across machines](teleport.md) covers [fan-out](teleport.md#fan-out), which sends one prompt to several agents and compares the results, and Smart Dispatch.
- `sheprd fan-out PROMPT` opens the fan-out dialog prefilled; see the [command line](cli.md).
