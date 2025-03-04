mod bighttp_hashes;
use anyhow::Result;
use bighttp::bighttp_hashes::BigHTTPHashes;
use clap::Parser;
use std::{fs::File, io::Write, path::PathBuf};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand, Debug)]
enum Commands {
    /// Generate chunked hash from an input file and save to a hashes file
    MakeHashesFile {
        /// Path to the input file
        input_file: PathBuf,
        /// Path to the output hashes file (defaults to input_file.hashes.bin)
        #[arg(short, long)]
        output_hashes_file: Option<PathBuf>,
    },
}

fn add_extension(mut path: PathBuf, extension: &str) -> PathBuf {
    if let Some(current_ext) = path.extension() {
        let mut new_ext = current_ext.to_string_lossy().to_string();
        new_ext.push_str(extension);
        path.set_extension(new_ext);
    } else {
        path.set_extension(extension);
    }
    path
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::MakeHashesFile {
            input_file,
            output_hashes_file,
        } => {
            let hashes: BigHTTPHashes<8> =
                BigHTTPHashes::from_file(&input_file, 1024 * 1024).unwrap();

            let output_file_path = match output_hashes_file {
                Some(path) => path,
                None => add_extension(input_file.clone(), ".hashes.bin"),
            };

            let mut output_file = File::create(output_file_path)?;
            output_file.write_all(&bitcode::encode(&hashes))?;
        }
    }

    Ok(())
}
