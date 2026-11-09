//! 侧栏里的文件夹：新建、重命名、删除、拖动排序，和新终端从哪个目录开始。

use super::*;

/// `~`、`~/xx`、`~\xx` 展开成主目录；空串是 `None`。`~名字` 是另一个用户的目录，这里不展开
pub(super) fn expand_home(text: &str) -> Option<PathBuf> {
    if text.is_empty() {
        return None;
    }
    let Some(rest) = text.strip_prefix('~') else {
        return Some(PathBuf::from(text));
    };
    let Some(home) = crate::store::user_home() else {
        return Some(PathBuf::from(text));
    };
    if rest.is_empty() {
        return Some(home);
    }
    if rest.starts_with(['/', '\\']) {
        Some(home.join(rest.trim_start_matches(['/', '\\'])))
    } else {
        Some(PathBuf::from(text))
    }
}

impl Workspace {
    /// 新建一个文件夹，名字空着就用「文件夹」「文件夹 2」这样排下去。这样的文件夹没有对应的目录
    pub fn create_folder(&self, name: &str) -> u64 {
        let name = name.trim();
        let name = if name.is_empty() {
            self.fresh_folder_name()
        } else {
            name.to_string()
        };
        self.push_folder(name, None)
    }

    pub(super) fn push_folder(&self, name: String, path: Option<PathBuf>) -> u64 {
        let id = self.next_folder.get();
        self.next_folder.set(id + 1);
        self.folders.update(|folders| {
            folders.push(Folder {
                id,
                name,
                path,
                open: true,
            });
        });
        id
    }

    /// 把一个磁盘目录加进「目标」（从访达拖进来、或者选的）：文件夹用目录的名字，
    /// 之后在里面创建的终端从这个目录起。这个目录已经有文件夹了就只是展开它。
    /// 不是目录（文件、不存在）返回 `None`
    pub fn add_directory(&self, path: &Path) -> Option<u64> {
        if !path.is_dir() {
            return None;
        }
        let same = |other: &Path| {
            other == path
                || matches!(
                    (other.canonicalize(), path.canonicalize()),
                    (Ok(a), Ok(b)) if a == b
                )
        };
        let existing = self
            .folders
            .get_untracked()
            .iter()
            .find(|folder| folder.path.as_deref().is_some_and(same))
            .map(|folder| folder.id);
        if let Some(id) = existing {
            self.reveal_folder(id);
            return Some(id);
        }
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        Some(self.push_folder(name, Some(path.to_path_buf())))
    }

    /// 弹出系统的选择目录对话框，选好的目录加进「目标」并在里面开一个终端
    pub fn choose_directory(&self) {
        let this = self.this.clone();
        // 对话框的任务不能记在按钮或菜单上：选完马上要换界面，它们的作用域会被卸掉
        aureole::detached(|| {
            aureole::spawn_local(async move {
                let mut dialog = aureole::FileDialog::new().title("选择目录");
                if let Some(home) = crate::store::user_home() {
                    dialog = dialog.directory(home);
                }
                let Some(path) = dialog.pick_directory().await else {
                    return;
                };
                if let Some(workspace) = this.upgrade()
                    && let Some(folder) = workspace.add_directory(&path)
                {
                    workspace.new_tab_in(folder);
                }
            })
        });
    }

    /// 还没被占用的默认文件夹名
    pub fn fresh_folder_name(&self) -> String {
        let names: Vec<String> = self
            .folders
            .get_untracked()
            .iter()
            .map(|folder| folder.name.clone())
            .collect();
        let mut candidate = "文件夹".to_string();
        let mut number = 2u32;
        while names.iter().any(|name| name == &candidate) {
            candidate = format!("文件夹 {number}");
            number += 1;
        }
        candidate
    }

    pub fn rename_folder(&self, id: u64, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        self.folders.update(|folders| {
            if let Some(folder) = folders.iter_mut().find(|folder| folder.id == id) {
                folder.name = name.to_string();
            }
        });
    }

    /// 删掉文件夹，里面的会话一起关掉
    pub fn delete_folder(&self, id: u64) {
        let sessions: Vec<u64> = self
            .tabs
            .get_untracked()
            .iter()
            .filter(|tab| tab.folder == Some(id))
            .map(|tab| tab.id)
            .collect();
        for session in sessions {
            self.close_tab(session);
        }
        self.folders
            .update(|folders| folders.retain(|folder| folder.id != id));
        if self.folder_prompt.get_untracked() == Some(FolderPrompt::Rename(id)) {
            self.folder_prompt.set(None);
        }
    }

    pub fn toggle_folder(&self, id: u64) {
        self.folders.update(|folders| {
            if let Some(folder) = folders.iter_mut().find(|folder| folder.id == id) {
                folder.open = !folder.open;
            }
        });
    }

    pub(super) fn reveal_folder(&self, id: u64) {
        let closed = self
            .folders
            .get_untracked()
            .iter()
            .any(|folder| folder.id == id && !folder.open);
        if !closed {
            return;
        }
        self.folders.update(|folders| {
            if let Some(folder) = folders.iter_mut().find(|folder| folder.id == id) {
                folder.open = true;
            }
        });
    }

    /// 新会话从哪个目录起：`explicit`（恢复上次时用关掉时的目录）、所在文件夹的目录、
    /// 设置里指定的起始目录，依次找第一个真是目录的；都没有就是 `None`（主目录）
    pub(super) fn start_directory(
        &self,
        folder: Option<u64>,
        explicit: Option<PathBuf>,
    ) -> Option<PathBuf> {
        let from_folder = folder.and_then(|id| {
            self.folders
                .get_untracked()
                .iter()
                .find(|candidate| candidate.id == id)
                .and_then(|candidate| candidate.path.clone())
        });
        let from_settings = (self.form.directory.get_untracked()
            == crate::settings::DirectoryKind::Custom)
            .then(|| expand_home(self.form.directory_path.get_untracked().trim()))
            .flatten();
        [explicit, from_folder, from_settings]
            .into_iter()
            .flatten()
            .find(|path| path.is_dir())
    }

    /// 文件夹在侧栏里排第几（从 0 起）
    pub fn folder_index(&self, id: u64) -> Option<usize> {
        self.folders
            .get_untracked()
            .iter()
            .position(|folder| folder.id == id)
    }

    /// 拖着的文件夹放下了：按拖动时算好的格子换顺序。顺序一变，落盘的 Effect 会自己存
    pub fn drop_folder(&self, id: u64) {
        let Some(dragged) = self
            .folder_drag
            .get_untracked()
            .filter(|dragged| dragged.id == id)
        else {
            return;
        };
        aureole::without_animation(|| {
            self.folder_drag.set(None);
            self.move_folder(id, dragged.to);
        });
    }

    /// 把文件夹挪到侧栏里第 `to` 个位置。已经在那里就什么也不做
    pub fn move_folder(&self, id: u64, to: usize) {
        let Some(from) = self.folder_index(id) else {
            return;
        };
        if from == to {
            return;
        }
        self.folders.update(|folders| {
            let moved = folders.remove(from);
            folders.insert(to.min(folders.len()), moved);
        });
    }
}
