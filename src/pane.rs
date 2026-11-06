//! 一个标签里终端的排布：一个终端，或者拆分出来的好几个。
//!
//! 是一棵二叉树：叶子是一个终端（会话），内部节点是一次拆分（左右并排或上下叠放，加一个分隔条的位置）。
//! 树本身是不可变的值：拆分、关掉一个都是得到一棵新树，界面认「结构」（[`Pane::signature`]）变了才重建，
//! 拖分隔条改的是节点里的 `ratio` 信号，不重建。

use std::rc::Rc;

use aureole::prelude::*;
use serde::{Deserialize, Serialize};

use crate::app::Session;

/// 怎么拆
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    /// 左右并排（左右分）
    Columns,
    /// 上下叠放（上下分）
    Rows,
}

/// 分隔条能拖到的范围：两边至少各留这么多
pub const RATIO_RANGE: (f32, f32) = (0.1, 0.9);

#[derive(Clone)]
pub enum Pane {
    Leaf {
        id: u64,
        session: Rc<Session>,
    },
    Split {
        axis: Axis,
        /// 第一块占多少（0 到 1）。信号放在不挂界面的作用域里：拆分是在点击、快捷键的处理器里做的，
        /// 记在它们的作用域上，一会儿就释放了
        ratio: Signal<f32>,
        first: Box<Pane>,
        second: Box<Pane>,
    },
}

impl Pane {
    pub fn leaf(id: u64, session: Rc<Session>) -> Self {
        Self::Leaf { id, session }
    }

    pub fn split_node(axis: Axis, ratio: f32, first: Pane, second: Pane) -> Self {
        Self::Split {
            axis,
            ratio: aureole::detached(|| signal(ratio)),
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    /// 所有终端，从左到右、从上到下（深度优先）
    pub fn leaves(&self) -> Vec<(u64, Rc<Session>)> {
        match self {
            Self::Leaf { id, session } => vec![(*id, session.clone())],
            Self::Split { first, second, .. } => {
                let mut leaves = first.leaves();
                leaves.extend(second.leaves());
                leaves
            }
        }
    }

    /// 第一个终端的编号
    pub fn first_id(&self) -> u64 {
        match self {
            Self::Leaf { id, .. } => *id,
            Self::Split { first, .. } => first.first_id(),
        }
    }

    pub fn session(&self, target: u64) -> Option<Rc<Session>> {
        self.leaves()
            .into_iter()
            .find(|(id, _)| *id == target)
            .map(|(_, session)| session)
    }

    pub fn contains(&self, target: u64) -> bool {
        self.leaves().iter().any(|(id, _)| *id == target)
    }

    /// 结构的指纹：谁在哪、怎么拆的。拖分隔条（`ratio`）不改它，拆分、关掉一个才改；界面靠它决定重不重建
    pub fn signature(&self) -> String {
        match self {
            Self::Leaf { id, .. } => id.to_string(),
            Self::Split {
                axis,
                first,
                second,
                ..
            } => {
                let mark = match axis {
                    Axis::Columns => '|',
                    Axis::Rows => '-',
                };
                format!("({}{mark}{})", first.signature(), second.signature())
            }
        }
    }

    /// 把终端 `target` 拆开：原来的在前（左边、上边），`added` 在后。`target` 不在树里就原样返回
    pub fn split(&self, target: u64, axis: Axis, added: Pane) -> Pane {
        match self {
            Self::Leaf { id, .. } if *id == target => {
                Self::split_node(axis, 0.5, self.clone(), added)
            }
            Self::Leaf { .. } => self.clone(),
            Self::Split {
                axis: own,
                ratio,
                first,
                second,
            } => {
                let (first, second) = if first.contains(target) {
                    (Box::new(first.split(target, axis, added)), second.clone())
                } else {
                    (first.clone(), Box::new(second.split(target, axis, added)))
                };
                Self::Split {
                    axis: *own,
                    ratio: *ratio,
                    first,
                    second,
                }
            }
        }
    }

    /// 去掉终端 `target`：它的兄弟补上它的位置。返回新的树和该聚焦到哪个终端（兄弟里第一个）。
    /// 只剩它一个（树就是它自己）或者不在树里时是 `None`
    pub fn remove(&self, target: u64) -> Option<(Pane, u64)> {
        let Self::Split {
            axis,
            ratio,
            first,
            second,
        } = self
        else {
            return None;
        };
        let is_target = |pane: &Pane| matches!(pane, Pane::Leaf { id, .. } if *id == target);
        if is_target(first) {
            return Some(((**second).clone(), second.first_id()));
        }
        if is_target(second) {
            return Some(((**first).clone(), first.first_id()));
        }
        let rebuilt = |first: Pane, second: Pane| Pane::Split {
            axis: *axis,
            ratio: *ratio,
            first: Box::new(first),
            second: Box::new(second),
        };
        if first.contains(target) {
            first
                .remove(target)
                .map(|(pane, focus)| (rebuilt(pane, (**second).clone()), focus))
        } else {
            second
                .remove(target)
                .map(|(pane, focus)| (rebuilt((**first).clone(), pane), focus))
        }
    }
}
