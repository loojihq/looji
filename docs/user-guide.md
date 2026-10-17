# looji user guide

looji is a desktop board for planning and tracking work. You keep **workspaces**, each with **projects**, each with a
**board** of tasks. You can also plan a project with AI, chat about a single task, link a project to a git repository, and
export a project as a PDF report.

This guide goes in the order you'll need things. If you've never used looji, follow [Your first task](#your-first-task)
and stop there. Everything else can wait.

- [Install and update](#install-and-update)
- [Your first task](#your-first-task)
- [Working on the board](#working-on-the-board)
- [Plan a project with AI](#plan-a-project-with-ai)
- [Chat about a task](#chat-about-a-task)
- [Connect a repository](#connect-a-repository)
- [Export a report](#export-a-report)
- [Updates](#updates)
- [Your data](#your-data)
- [Troubleshooting](#troubleshooting)

## Install and update

Download `looji-<version>-windows-x64-setup.exe` from the project's
[releases page](https://github.com/danielkosgei/looji/releases) and run it. It installs for your own account, so it
doesn't ask for administrator rights. The first time you start looji, a short tour walks through the main screens. You can
skip it, and bring it back later with **Welcome tour: show again** in the sidebar.

## Your first task

A new install has one workspace, called **Personal**, with no projects. To make your first task:

1. **Create a project.** In the sidebar, choose **Create project** (or **+ New project**). Type a name, such as
   `My first project`, and press **Enter**. The description is optional.
2. **Add a task.** Above the board, on the right, choose **+ New task**. Type a title, such as `Write the project brief`,
   and choose **Save**. Only the title is required.
3. **See it on the board.** The card appears in the **To do** column.

That is the whole loop: workspaces hold projects, and projects hold tasks on a board.

To keep work apart, make another workspace with **+ New workspace** at the top of the sidebar, type its name, and press
**Enter**. The sidebar also lets you rename a workspace, give it an icon, or delete it.

## Working on the board

Every project has a board with five columns: **Backlog**, **To do**, **In progress**, **In review**, and **Done**. The
count beside each name is how many tasks it holds. **In review column: shown** hides or shows the In review column.

### Move and reorder cards

- **With the mouse:** press and drag a card to another column, or to a new place in the same column. A dashed outline shows
  where it will land.
- **With the keyboard:** press **Tab** until the card is focused, then **Space** to pick it up. The arrow keys move it,
  **Enter** drops it, and **Escape** cancels. Nothing is saved until you press Enter.

A card's order within a column is kept, so what you arrange is what you see next time.

### Edit a task

Click a card to open its panel on the right. There you can change its status, priority, description, tags, estimate, dates,
and assignee, and see its history. **Edit details** opens the full form, which has the same fields as **+ New task**:

| Field | What it's for |
| --- | --- |
| Title | Required. Shown on the card. |
| Description | Longer notes. |
| Priority | None, Low, Medium, or High. |
| Estimate, in hours | For example `2.5`. Used to work out a due date. |
| Start date, Due date | Written as `YYYY-MM-DD`. |
| Assignee | Someone from the project's team. |
| Tags | Type a tag and press comma or Enter. |

When a task moves into **In progress** for the first time and has an estimate, looji sets its due date from your
**Hours**, the working days and times you set for the project.

### Find things

Above the board, **Filters** narrows the cards by status, priority, assignee, or tag, and **Clear filters** puts them all
back. **Search cards** (press **Ctrl+F**) looks through titles and descriptions as you type.

### Other views

The buttons at the top switch the view of the same tasks: **Board**, **List** (a sortable table), **Roadmap** (tasks on a
timeline by their dates), and **Workload** (how much is assigned to each person). **Team** manages the people you can
assign tasks to, and **Hours** sets working days and times.

## Plan a project with AI

**Plan** turns a goal into a plan: a short spec, user stories, and a backlog of tasks.

1. Choose **Plan**. Describe what you want to build, pick the team members who'll do it, and optionally set a target date.
2. looji writes the plan as it streams in. You can stop it at any point.
3. Review the result. Edit any title, priority, estimate, or assignee, add or remove tasks, or ask for a section to be
   rewritten with a follow-up instruction. Edits are saved as you make them.
4. **Publish** adds the tasks to the project's backlog.

**Drafts** lists your plans, including ones you haven't finished. You can open one to carry on, copy it, or delete it. A
published plan is read-only.

Planning needs an Anthropic API key. See [Add your API key](#add-your-api-key).

### Add your API key

looji keeps the key in Windows Credential Manager, not in a file. A screen for entering it inside looji isn't there yet. Until
it is, add it from a terminal. Replace `YOUR-KEY` with your key:

```
cmdkey /generic:anthropic-api-key.looji /user:anthropic-api-key /pass:YOUR-KEY
```

To remove it again: `cmdkey /delete:anthropic-api-key.looji`.

## Chat about a task

Open a task and choose **Chat** in its panel. You can ask about the task, and the conversation is saved with it, so it's
there next time.

- **Replies stream in** as they're written. **Stop** ends a reply where it is and keeps what arrived.
- If the connection drops, the reply is marked as stopped early and keeps its text. Choose **Retry** to ask again.
- **Model** chooses which model answers. It's remembered for the task, and a change applies to your next message.
- **Repository code** adds code from the project's connected repository to what the model sees, picked by how well it
  matches the task. It's only available once a [repository is connected](#connect-a-repository).
- Replies can include headings, lists, tables, links, and code. Each code block has a **Copy** button that copies exactly the
  code.

Chat needs an API key too. See [Add your API key](#add-your-api-key).

## Connect a repository

Link a project to a git repository on your computer so chat can use its code.

1. Choose **Repository**.
2. Paste the folder's path and press **Enter**, or choose **Browse**.
3. looji checks that it is a git repository and shows **Connected**.

The folder must be the top of the repository, the one that contains `.git`, not a folder inside it. If the folder is moved
or deleted later, the status shows **Broken** and keeps the path, so you can **Locate again** or put the folder back. Only
**Disconnect** removes the path.

### Repositories in WSL

A repository inside a WSL distribution works too. Use its Windows path, such as
`\\wsl.localhost\Ubuntu\home\you\project`. If the distribution is stopped, looji says so and offers **Start Ubuntu**, because
looking inside a stopped distribution would otherwise start it without asking.

## Export a report

Choose **Export PDF** above the board. Pick where to save it in the dialog. The report has a cover, a summary with a count for
each status, and then every task grouped by status in board order. Statuses with no tasks still appear, with a note.
Cancelling the dialog does nothing. If looji can't write to the place you chose, it says so in the sidebar.

## Updates

looji checks for a new version when it starts, and again each day. If there's one, a notice appears in the bottom corner:

- **Update now** downloads the new version, checks it against the checksum published with the release, and restarts into it.
  Your data stays as it was. A file that fails the check is deleted and never run.
- **View release** opens the release notes. **Dismiss** hides the notice.

If looji can't reach GitHub it says nothing and tries again later. Before a new version changes your database, looji copies
it to the `backups` folder, described below.

## Your data

Everything is in one folder, `%APPDATA%\looji`:

| File or folder | What it is |
| --- | --- |
| `looji.db` | Your workspaces, projects, tasks, chats, and plans. |
| `icons` | Workspace icons. |
| `backups` | A copy of the database from before the last upgrade. |
| `updates` | A downloaded update, until it's installed. |

To move looji to another computer, close it and copy `looji.db`. Your API key is in Windows Credential Manager and isn't part
of that folder.

## Troubleshooting

**Plan or Chat says there's no API key.** Add it as described in [Add your API key](#add-your-api-key).

**A repository shows Broken.** The folder has moved or been deleted. Choose **Locate again**, or put it back. The path is kept.

**A WSL repository says the distribution is stopped.** Choose **Start** next to its name.

**An update didn't install.** The message says why. If the download was interrupted, **Try again** carries on from where it
stopped. If the file failed its check, it was deleted. Try again later, or install from the releases page.

**looji won't start after an upgrade.** Close it. In `%APPDATA%\looji`, delete `looji.db-wal` and `looji.db-shm` if they
exist, then copy the newest file in the `backups` folder over `looji.db`. That's your data from before the upgrade. The two
extra files belong to the newer database, so they have to go first. Then install the previous version from the releases page.
