#![feature(if_let, macro_rules, phase, slicing_syntax)]
 
extern crate curl;
extern crate docopt;
extern crate libc;
extern crate regex;
#[phase(plugin)] extern crate regex_macros;
extern crate serialize;
extern crate term;

use curl::http;
use docopt::Docopt;
use error::Error;
use serialize::{Decodable, Decoder, json};
use std::collections::HashSet;
use std::io::process::{Command, InheritFd};
use std::io::stdio;
use std::str;

mod error;

#[deriving(Decodable, Show)]
struct Args {
    arg_repository: String,
    flag_min: Option<uint>
}

#[deriving(Decodable)]
struct Repository {
    git_url: String
}

#[deriving(Decodable)]
struct User {
    login: String
}

struct Commit {
    user: User,
    repo: Option<Repository>,
    sha: String,
    ref_: String
}

// Hack to work-around the fact that `ref` is a keyword.
impl <D: Decoder<E>, E> Decodable<D, E> for Commit {
    fn decode(__arg_0: &mut D) -> Result<Commit, E> {
        __arg_0.read_struct("Commit", 4u, |_d|
            Ok(Commit {
                user: try!(_d.read_struct_field("user", 0u, |_d| Decodable::decode(_d))),
                repo: try!(_d.read_struct_field("repo", 1u, |_d| Decodable::decode(_d))),
                sha: try!(_d.read_struct_field("sha", 2u, |_d| Decodable::decode(_d))),
                ref_: try!(_d.read_struct_field("ref", 3u, |_d| Decodable::decode(_d)))
            })
        )
    }
}

#[deriving(Decodable)]
struct PullRequest {
    number: uint,
    title: String,
    head: Commit,
    body: Option<String>
}

pub type Response = Vec<PullRequest>;

const BORS_STATUS_URL: &'static str = "http://buildbot.rust-lang.org/bors/bors-status.js";
fn get_approved_prs() -> Result<HashSet<uint>, Error> {
    println!("fetching approved PRs...");

    let response = try!(http::handle()
        .get(BORS_STATUS_URL)
        .header("User-Agent", "bors roll-up")
        .exec());
    let string = try!(
        str::from_utf8(response.get_body())
            .ok_or(json::ApplicationError("Non UTF-8 response".to_string()))
    );
    let index = try!(string.find_str("var bors =")
        .ok_or(json::ApplicationError("Unexpected status JSON shape".to_string())));

    let json_str = string[(index + "var bors =".len())..string.len() - 2];
    let approved_prs = try!(json::decode::<Vec<PullRequestStatus>>(json_str));
    Ok(approved_prs
        .into_iter()
        .filter(|pr| pr.state.as_slice() == "APPROVED")
        .map(|pr| pr.num)
        .collect())
}

#[deriving(Decodable)]
struct PullRequestStatus {
    num: uint,
    state: String
}

fn get_next_page_url(response: &http::Response) -> Option<String> {
    static REGEX: regex::Regex = regex!(r##"<([^>]*)>; rel="next""##);
    response.get_header("link")
        .iter()
        .flat_map(|header| header.as_slice().split(','))
        .filter_map(|link| REGEX.captures(link))
        .map(|captures| captures.at(1).to_string())
        .next()
}

fn fetch_page(url: &str) -> Result<Vec<PullRequest>, Error> {
    println!("fetching -- {}", url);

    let response = try!(http::handle()
        .get(url.as_slice())
        .header("User-Agent", "bors roll-up")
        .exec());
    let string = try!(
        str::from_utf8(response.get_body())
            .ok_or(json::ApplicationError("Non UTF-8 response".to_string()))
    );
    let mut page_prs = try!(json::decode::<Vec<PullRequest>>(string));
    if let Some(url) = get_next_page_url(&response) {
        let next_page = try!(fetch_page(url.as_slice()));
        page_prs.extend(next_page.into_iter());
    }
    Ok(page_prs)
}

fn fetch(repository: &str) -> Result<Vec<PullRequest>, Error> {
    let url: String = format!("https://api.github.com/repos/{}/pulls?direction=asc", repository);
    fetch_page(url.as_slice())
}
 
macro_rules! git(
    ($($a:expr),*) => ({
        let mut cmd = Command::new("git");
        $(cmd.arg($a);)*
        println!("\x1b[38;5;106m$ {}\x1b[0m", cmd);
        cmd.stdout(InheritFd(libc::STDOUT_FILENO));
        cmd.stderr(InheritFd(libc::STDERR_FILENO));
        cmd.status()
    })
)

enum Prompt {
    Yes,
    No,
    Quit
}

fn get_prompt(prompt: &str) -> Prompt {
    let mut input = stdio::stdin();
    loop {
        print!("{}\n[y/N/q]: ", prompt);
        let line = input.read_line().unwrap();
        match line.as_slice().trim() {
            "y" | "Y" => return Yes,
            "n" | "N" => return No,
            "q" | "Q" => return Quit,
            _ => continue
        }
    }
}

fn merge_pull_request(pull_request: PullRequest) -> Result<(), Error> {
    let PullRequest {
        head: Commit {
            user: User { login },
            repo: repository,
            ref_,
            sha
        },
        number,
        body,
        ..
    } = pull_request;

    let git_url = match repository {
        Some(Repository { git_url }) => git_url,
        None => return Ok(())
    };

    let message = format!("rollup merge of #{}: {}/{}\r\n\r\n{}",
        number, login, ref_, body.as_ref().map_or("", |s| s.as_slice()));
    try!(git!("remote", "rm", login.as_slice()));
    try!(git!("remote", "add", login.as_slice(), git_url));
    try!(git!("fetch", login.as_slice(), ref_));
    try!(git!("merge", "--no-ff", "-m", message, sha)
        .or_else(|_| {
            println!("\x1b[38;5;160m{}\x1b[0m", "couldn't merge");
            git!("merge", "--abort")
        }));
    try!(git!("remote", "rm", login.as_slice()));
    Ok(())
}

static USAGE: &'static str = "
Usage: rollup <repository>

Options:
    -m, --min   # of the oldest PR to include.
";

fn run() -> Result<(), Error> {
    let docopt = try!(Docopt::new(USAGE));
    let args: Args = docopt.decode().unwrap_or_else(|e| e.exit());
    let repository_name = args.arg_repository.as_slice();

    let approved = try!(get_approved_prs());
    for pull_request in (try!(fetch(repository_name)))
        .into_iter()
        .filter(|pr| approved.contains(&pr.number))
        .filter(|pr| pr.number >= args.flag_min.unwrap_or(0)) {
        match get_prompt(format!("merge #{} \"{}\"?", pull_request.number, pull_request.title).as_slice()) {
            Yes => (),
            No => continue,
            Quit => break
        }
        try!(merge_pull_request(pull_request));
    }
    Ok(())
}

fn main() {
    run().unwrap();
}
