//! PTY 会话：spawn 子进程、读写与尺寸同步；每个窗格一个会话。

use std::io::{self, Read, Write};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::layout::PaneId;

/// PTY 侧事件：读线程经回调交给事件循环。
#[derive(Debug)]
pub enum PtyEvent {
    /// 子进程输出。
    Output(Vec<u8>),
    /// 子进程退出或输出流关闭。
    Exited,
}

/// 单个窗格的 PTY 会话。
pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    size: PtySize,
}

impl PtySession {
    /// 启动 `$SHELL`；输出由专属读线程经 `on_event` 回传。
    pub fn spawn(
        id: PaneId,
        cols: u16,
        rows: u16,
        on_event: impl Fn(PtyEvent) + Send + 'static,
    ) -> io::Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(to_pty_size(cols, rows))
            .map_err(io::Error::other)?;

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let mut command = CommandBuilder::new(shell);
        command.env("TERM", "xterm-256color");
        if let Ok(cwd) = std::env::current_dir() {
            command.cwd(cwd);
        }
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(io::Error::other)?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
        let writer = pair.master.take_writer().map_err(io::Error::other)?;

        std::thread::Builder::new()
            .name(format!("pty-{}", id.raw()))
            .spawn(move || {
                let mut buffer = [0u8; 8192];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(read) => on_event(PtyEvent::Output(buffer[..read].to_vec())),
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                on_event(PtyEvent::Exited);
            })
            .map_err(io::Error::other)?;

        Ok(Self {
            master: pair.master,
            writer,
            child,
            size: to_pty_size(cols, rows),
        })
    }

    /// 写入按键/粘贴字节。
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    /// 同步 PTY 窗口尺寸。
    pub fn resize(&mut self, cols: u16, rows: u16) -> io::Result<()> {
        let size = to_pty_size(cols, rows);
        if size.rows == self.size.rows && size.cols == self.size.cols {
            return Ok(());
        }
        self.master.resize(size).map_err(io::Error::other)?;
        self.size = size;
        Ok(())
    }

    /// 终止子进程；重复调用安全。
    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        self.kill();
    }
}

fn to_pty_size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}
