#![allow(irrefutable_let_patterns)]

mod cli;
mod webserver;

#[actix_web::main]
pub async fn main() {
    let options = cli::parse();

    // Start the werbserver on --stat/-s
    if options.start {
        webserver::start().await.expect("webserver failed");
        return;
    }

    // User didn't read the docs
    println!("Nothing to do. Use `-s` to start the dictionary");
}
