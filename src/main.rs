// #![feature(try_trait)]
use crate::utils::{fstderr, fstdout};
use builder::FlakyFinderBuilder;
use error::FlakyFinderResult;
use indicatif::{ProgressBar, ProgressIterator, ProgressStyle};
use std::{
    io::{stdout, Write},
    process::{Command, ExitStatus, Output},
};

mod builder;
mod cli;
mod error;
mod utils;

#[derive(Debug)]
pub(crate) struct FlakyFinder {
    /// The actual command that we need to test.
    cmd: String,
    /// The status of the process we are currently evaluating
    exit_status: Option<ExitStatus>,
    /// The output from the process we are evaluating: stdout/stderr
    outputs: Vec<Output>,
    /// Let's run those tests in parallel
    nb_threads: u32,
    /// How many times we should run the command.
    runs: u64,
    /// Shall we stop on the first flaky test found or continue
    should_continue: bool,
    /// Should we show the errors as they come or only in the end as a summary
    show_errors_as_summary: bool,
}

impl FlakyFinder {
    /// Runs a command multiple time trying to find if it can fail at some point.
    pub(crate) fn run(&mut self) -> FlakyFinderResult<bool> {
        let runs = self.runs;
        let nb_threads = self.nb_threads;
        let cmd = &self.cmd;

        // Provide a custom bar style
        let pb = ProgressBar::new(runs);
        pb.set_style(ProgressStyle::default_bar().template(
                "{spinner:.cyan} [{elapsed_precise}] [{bar:40.white/gray}] ({pos}/{len}, ETA {eta}) {msg}",
        ));

        let (sx, rx) = crossbeam_channel::bounded(runs as usize);

        let cmd = std::sync::Arc::new(cmd.to_string());

        // Execute the process at least one time in order to single process the compilation
        print!(">> Warming up...");
        stdout().flush()?;
        let _ = Command::new("sh")
            .arg("-c")
            .arg(cmd.to_string())
            .output()
            .expect("Fail to warming up.");
        println!("done.");

        let pool = threadpool::ThreadPool::new(nb_threads as usize);

        for _ in 0..runs {
            let cmd = cmd.clone();
            let sx = sx.clone();

            pool.execute(move || {
                let output = Command::new("sh")
                    .arg("-c")
                    .arg(cmd.to_string())
                    .output()
                    .expect("Fail to run command process.");

                sx.send(output)
                    .expect("Fail to send Command's output to channel.");
            });
        }

        drop(sx);

        let mut error_counter = 0;
        for recv_output in rx.iter().progress_with(pb.clone()) {
            let status = recv_output.status;
            if !status.success() {
                error_counter += 1;

                if !self.should_continue {
                    break;
                } else {
                    pb.set_message(&format!(
                        "-- {:.*}% Errors found...",
                        1,
                        self.percent_of_error_found(error_counter)
                    ));
                    pb.inc(0);

                    if !self.show_errors_as_summary {
                        pb.println(::std::str::from_utf8(&recv_output.stdout)?);
                        pb.println(::std::str::from_utf8(&recv_output.stderr)?);
                        pb.println(format!(
                            "\n{:^80}\n\n",
                            "##########################################"
                        ));
                    }
                }
                self.outputs.push(recv_output.clone());
            }
        }

        drop(rx);

        if error_counter > 1 {
            pb.finish();
        }

        if self.show_errors_as_summary {
            self.show_errors()?;
        }

        Ok(self.outputs.is_empty())
    }

    /// Print out all the errors we found.
    fn show_errors(&self) -> FlakyFinderResult<()> {
        if self.outputs.is_empty() {
            eprintln!(">> Nothing found 👍");
        } else {
            eprintln!(
                "\n>> {:.*}% Errors found:",
                1,
                self.percent_of_error_found(self.outputs.len() as u64)
            );
        }
        for error_output in self.outputs.iter() {
            fstdout(&error_output.stdout)?;
            fstderr(&error_output.stderr)?;
            if self.outputs.len() > 1 {
                eprintln!("\n{:^80}\n", "##########################################");
            }
        }
        Ok(())
    }

    fn percent_of_error_found(&self, nb_errors: u64) -> f32 {
        (nb_errors as f32 / self.runs as f32) * 100.0
    }
}

fn main() -> std::result::Result<(), &'static str> {
    let mut ff = FlakyFinderBuilder::from_cli()
        .expect("Failed to build")
        .build();
    if ff.runs < 1 {
        panic!("Number of 'runs' has to be > 0.")
    }

    let good = ff.run().expect("Fail to processes.");
    if !good {
        return Err("Flaky tests found");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_test() {
        let _cmd = "cargo test -- --nocapture release_test";
        let cmd = "ls";
        let _ff = FlakyFinderBuilder::new().cmd(cmd).nb_threads(1).build();

        // assert_eq!(true, false);
    }

    #[test]
    #[should_panic]
    fn failing_test() {
        assert!(false);
    }
}
