use std::fs::File;
use std::io::{stderr, stdin, stdout, ErrorKind, Read, Write};
use std::mem::zeroed;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::thread::sleep;
use std::time::Duration;

use anyhow::{anyhow, Result};
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::libc::{_exit, pause, winsize, TIOCGWINSZ, TIOCSWINSZ};
use nix::pty::{forkpty, Winsize};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{close, pipe};
use nix::{ioctl_read_bad, ioctl_write_ptr_bad, sys};
use rust_apt::progress::{AcquireProgress, DynInstallProgress, InstallProgress};
use rust_apt::util::{get_apt_progress_string, terminal_height, terminal_width};
use rust_apt::Cache;
use tokio::sync::mpsc;

use crate::config::Config;

pub enum Message {
	Yes(String),
	No(String),
}

pub struct NalaInstallProgress {
	tx: mpsc::UnboundedSender<Message>,
}

impl<'a> NalaInstallProgress {
	pub fn new(tx: mpsc::UnboundedSender<Message>) -> Self { Self { tx } }
}

impl DynInstallProgress for NalaInstallProgress {
	fn status_changed(
		&mut self,
		pkgname: String,
		_steps_done: u64,
		_total_steps: u64,
		action: String,
	) {
		self.tx
			.send(Message::Yes(format!("{action}:{pkgname} from thread!")))
			.unwrap();
	}

	// TODO: Need to figure out when to use this.
	fn error(&mut self, _pkgname: String, _steps_done: u64, _total_steps: u64, _error: String) {}
}

// Define the ioctl read call for TIOCGWINSZ
ioctl_read_bad!(tiocgwinsz, TIOCGWINSZ, winsize);

// Define the ioctl write call for TIOCSWINSZ
ioctl_write_ptr_bad!(tiocswinsz, TIOCSWINSZ, winsize);

fn get_winsize(fd: RawFd) -> nix::Result<winsize> {
	let mut ws: winsize = unsafe { zeroed() };
	unsafe { tiocgwinsz(fd, &mut ws) }?;
	Ok(ws)
}

fn set_winsize(fd: RawFd, rows: u16, cols: u16) -> nix::Result<()> {
	let ws = winsize {
		ws_row: rows,
		ws_col: cols,
		ws_xpixel: 0,
		ws_ypixel: 0,
	};
	unsafe { tiocswinsz(fd, &ws) }?;
	Ok(())
}

pub fn run_install(cache: Cache) -> Result<()> {
	println!("run_install");

	let (statusfd, writefd) = pipe()?;
	fcntl(statusfd.as_raw_fd(), FcntlArg::F_SETFL(OFlag::O_NONBLOCK))?;

	let window_size = get_winsize(stdin().as_raw_fd())?;
	println!("start fork");

	match unsafe { forkpty(&window_size, None)? } {
		nix::pty::ForkptyResult::Child => {
			close(statusfd.as_raw_fd())?;

			let mut progress = AcquireProgress::apt();

			let mut inst_progress = InstallProgress::fd(writefd.as_raw_fd());

			cache.commit(&mut progress, &mut inst_progress)?;
			close(writefd.as_raw_fd())?;
			unsafe { _exit(0); }
		},
		nix::pty::ForkptyResult::Parent { child, master } => {
			let mut buffer = [0u8; 4096];
			let mut stat_buff = [0u8; 4096];
			let mut master_file = unsafe { File::from_raw_fd(master.as_raw_fd()) };
			let mut status = unsafe { File::from_raw_fd(statusfd.as_raw_fd()) };

			loop {
				// Wait for the child process to finish and get its exit code
				let wait_status = waitpid(child, Some(WaitPidFlag::WNOHANG))?;
				if let WaitStatus::Exited(_, exit_code) = wait_status {
					if exit_code != 0 {
						std::process::exit(exit_code);
					}
				}

				match master_file.read(&mut buffer) {
					Ok(0) => break, // EOF
					Ok(n) => {
						// std::io::stdout().write_all(&buffer[..n]).unwrap();
						// std::io::stdout().flush().unwrap();
					},
					Err(ref e) if e.kind() == ErrorKind::WouldBlock => {},
					// EOF
					Err(ref e) if e.raw_os_error().is_some_and(|code| code == 5) => {
						break;
					},
					Err(e) => return Err(anyhow!(e)),
				}

				match status.read(&mut stat_buff) {
					Ok(0) => break, // EOF
					Ok(n) => {
						std::io::stdout().write_all(&stat_buff[..n]).unwrap();
						std::io::stdout().flush().unwrap();
					},
					Err(ref e) if e.kind() == ErrorKind::WouldBlock => {},
					Err(ref e) if e.raw_os_error().is_some_and(|code| code == 5) => {
						break;
					},
					Err(e) => return Err(anyhow!(e)),
				}


			}
		},
	}

	Ok(())
}

// fn read_exact<Fd: AsFd>(f: Fd, buf: &mut [u8]) {
//     let mut len = 0;
//     while len < buf.len() {
//         // get_mut would be better than split_at_mut, but it requires nightly
//         let (_, remaining) = buf.split_at_mut(len);
//         len += nix::unistd::read(&f, remaining).unwrap();
//     }
// }
// pub struct Pty {
// 	fd: RawFd,
// 	cache: Cache,
// }

// impl Pty {
// 	fn new(cache: Cache) -> Result<Pty> {
// 		let (statusfd, writefd) = pipe()?;

// 		let window_size = get_winsize(stdin().as_raw_fd())?;
// 		match unsafe { forkpty(&window_size, None)? } {
// 			nix::pty::ForkptyResult::Child => {
// 				let mut progress = AcquireProgress::apt();
// 				let inst_progress = InstallProgress::fd();

// 				cache.commit(&mut progress, &mut InstallProgress::new(acquire))?;
// 			},
// 			nix::pty::ForkptyResult::Parent { child, master } => todo!(),
// 		}

// 		// TODO: Do we need termios things? I think not.
// 		// match unsafe { forkpty(&window_size, None)? } {
// 		// 	nix::pty::ForkptyResult::Parent { child, master } => todo!(),
// 		// 	nix::pty::ForkptyResult::Child => todo!(),
// 		// }

// 		Ok(Pty { fd, cache })
// 	}

// 	pub fn is_alive(&self) -> bool {
// 		sys::stat::fstat(self.fd).is_ok()
// 	}
// }
