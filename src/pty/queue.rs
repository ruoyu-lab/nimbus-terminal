//! pty 到主线程的那条道。
//!
//! 框架的公开 API 里没有**跨线程送回主线程并且能带进主线程专属状态**的通道：
//! `main_thread_sender` 投的闭包要求 `Send`，而 `CharGrid`、`Terminal` 里都是 `Rc`，带不进去。
//! 所以这里自己写一个最小的队列：后台线程往里塞，主线程 `await`，塞的时候把等着的那个 future 叫醒。
//!
//! # 背压
//!
//! 队列有容量（[`MAX_QUEUED`]）：界面来不及处理时（`yes`、`cat` 一个大文件），读线程在这里停下，
//! 内核的 pty 缓冲写满之后，子进程自己就被堵住了。没有这个上限，读线程会一直读、内存一直涨，
//! 而界面这边永远追不上。

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

/// 队列里最多攒这么多字节，再多读线程就等着
pub const MAX_QUEUED: usize = 1 << 20;

/// 从 pty 读到的一块字节，或者「子进程退出了」
#[derive(Clone, Debug)]
pub enum Chunk {
    Bytes(Vec<u8>),
    /// 带着退出码
    Closed(i32),
}

struct State {
    chunks: VecDeque<Chunk>,
    /// 队列里一共攒了多少字节
    queued: usize,
    waker: Option<Waker>,
    /// 子进程退出了，不会再有新的字节
    closed: bool,
    /// 取的一端没了（窗口关了），塞的一端该收摊
    abandoned: bool,
}

struct Shared {
    state: Mutex<State>,
    /// 取走了东西、有空位了
    space: Condvar,
}

impl Shared {
    /// 锁坏了（别的线程在持锁时 panic）也照样用里面的数据：队列的状态每一步都是自洽的
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// 往里塞的一端，给读字节的后台线程
pub struct Producer {
    shared: Arc<Shared>,
}

impl Producer {
    /// 塞一块进去，叫醒正在等的未来。队列满了就等到有空位；取的那头已经没了（窗口关了）就当丢掉。
    /// 返回 `false` 表示该停了（取的那头没了）
    pub fn push(&self, chunk: Chunk) -> bool {
        let waker = {
            let mut state = self.shared.lock();
            if let Chunk::Bytes(bytes) = &chunk {
                // 单块比上限还大也放行（上限是「已经攒了这么多就别再塞」，不是单块大小）
                while state.queued >= MAX_QUEUED && !state.abandoned && !state.closed {
                    state = self
                        .shared
                        .space
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
                if state.closed {
                    return !state.abandoned;
                }
                state.queued += bytes.len();
            }
            if state.abandoned {
                return false;
            }
            if matches!(chunk, Chunk::Closed(_)) {
                state.closed = true;
            }
            state.chunks.push_back(chunk);
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        true
    }
}

/// 取的那一端，主线程用。丢掉它，读线程就知道该收摊了
pub struct ByteQueue {
    shared: Arc<Shared>,
}

impl ByteQueue {
    pub fn pair() -> (Self, Producer) {
        let state = State {
            chunks: VecDeque::new(),
            queued: 0,
            waker: None,
            closed: false,
            abandoned: false,
        };
        let shared = Arc::new(Shared {
            state: Mutex::new(state),
            space: Condvar::new(),
        });
        (
            Self {
                shared: shared.clone(),
            },
            Producer { shared },
        )
    }

    /// 现在就取一块，没有就是 `None`（不等）。一次处理一批、处理完再画一次，靠它
    pub fn try_next(&self) -> Option<Chunk> {
        let mut state = self.shared.lock();
        let chunk = state.chunks.pop_front()?;
        if let Chunk::Bytes(bytes) = &chunk {
            state.queued -= bytes.len();
            self.shared.space.notify_one();
        }
        Some(chunk)
    }

    /// 下一块；子进程退出且都取完之后是 `None`
    pub fn next(&self) -> Next<'_> {
        Next { queue: self }
    }
}

impl Drop for ByteQueue {
    fn drop(&mut self) {
        self.shared.lock().abandoned = true;
        self.shared.space.notify_all();
    }
}

pub struct Next<'q> {
    queue: &'q ByteQueue,
}

impl Future for Next<'_> {
    type Output = Option<Chunk>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        // 取和登记 waker 在同一把锁里做：读线程不会在两步之间塞进一块而没人叫醒
        let mut state = self.queue.shared.lock();
        if let Some(chunk) = state.chunks.pop_front() {
            if let Chunk::Bytes(bytes) = &chunk {
                state.queued -= bytes.len();
                self.queue.shared.space.notify_one();
            }
            return Poll::Ready(Some(chunk));
        }
        if state.closed {
            return Poll::Ready(None);
        }
        state.waker = Some(context.waker().clone());
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// 手工把 future 推一步（测试里没有事件循环，直接 `poll`）
    fn poll_once(queue: &ByteQueue) -> Option<Chunk> {
        let mut future = Box::pin(queue.next());
        let mut context = Context::from_waker(Waker::noop());
        match future.as_mut().poll(&mut context) {
            Poll::Ready(chunk) => chunk,
            Poll::Pending => panic!("队列里还有东西，不该等"),
        }
    }

    #[test]
    fn push_before_await_is_not_lost() {
        let (queue, producer) = ByteQueue::pair();
        producer.push(Chunk::Bytes(b"hi".to_vec()));
        assert!(matches!(poll_once(&queue), Some(Chunk::Bytes(_))));
    }

    /// 子进程退出以后：先把手里已经读到的交完，再说没有了
    #[test]
    fn closed_ends_the_queue_after_the_backlog() {
        let (queue, producer) = ByteQueue::pair();
        producer.push(Chunk::Bytes(b"a".to_vec()));
        producer.push(Chunk::Closed(0));
        producer.push(Chunk::Bytes("退出之后的不算".as_bytes().to_vec()));
        assert!(matches!(poll_once(&queue), Some(Chunk::Bytes(_))));
        assert!(matches!(poll_once(&queue), Some(Chunk::Closed(0))));
        assert!(poll_once(&queue).is_none());
    }

    /// 队列满了读线程要停下来等，取走一块之后才继续：这是 `yes` 刷屏时内存不涨的前提
    #[test]
    fn a_full_queue_blocks_the_producer_until_the_consumer_drains() {
        let (queue, producer) = ByteQueue::pair();
        let (done_sender, done) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            // 第一块就把队列填到上限，第二块必须等
            producer.push(Chunk::Bytes(vec![0; MAX_QUEUED]));
            producer.push(Chunk::Bytes(vec![1; 16]));
            done_sender.send(()).ok();
        });
        assert!(
            done.recv_timeout(Duration::from_millis(200)).is_err(),
            "队列满了，第二次 push 不该返回"
        );
        assert!(matches!(queue.try_next(), Some(Chunk::Bytes(bytes)) if bytes.len() == MAX_QUEUED));
        done.recv_timeout(Duration::from_secs(5))
            .expect("取走之后读线程应该继续");
        handle.join().ok();
        assert!(matches!(queue.try_next(), Some(Chunk::Bytes(bytes)) if bytes.len() == 16));
    }

    /// 窗口关了（取的一端丢掉）：卡在满队列上的读线程要醒过来，并且被告知该停了
    #[test]
    fn dropping_the_queue_releases_a_blocked_producer() {
        let (queue, producer) = ByteQueue::pair();
        let handle = std::thread::spawn(move || {
            producer.push(Chunk::Bytes(vec![0; MAX_QUEUED]));
            producer.push(Chunk::Bytes(vec![1; 16]))
        });
        std::thread::sleep(Duration::from_millis(100));
        drop(queue);
        assert!(!handle.join().expect("读线程不该 panic"), "该停了");
    }
}
