use crate::hashes::BigHTTPHashes;
use anyhow::Result;
use reqwest::Client;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
use tokio::sync::mpsc;
use url::Url;

#[derive(Default)]
pub struct BigHttpClient<const HASH_SIZE: usize> {}

impl<const HASH_SIZE: usize> BigHttpClient<HASH_SIZE> {
    pub async fn update_file(
        &self,
        remote_hashes: &BigHTTPHashes<HASH_SIZE>,
        file_url: &Url,
        output_file: &PathBuf,
        progress_tx: Option<mpsc::Sender<usize>>,
    ) -> Result<()> {
        // Generate hashes for the local file if it exists
        let local_hashes = if output_file.exists() {
            let lh: BigHTTPHashes<HASH_SIZE> =
                BigHTTPHashes::from_file(output_file, remote_hashes.chunk_size)?;
            if remote_hashes.file_size_bytes() == lh.file_size_bytes() {
                lh
            } else {
                std::fs::remove_file(&output_file)?;
                BigHTTPHashes::noised(remote_hashes.chunk_size, remote_hashes.file_size_bytes())
            }
        } else {
            BigHTTPHashes::noised(remote_hashes.chunk_size, remote_hashes.file_size_bytes())
        };

        if local_hashes == *remote_hashes {
            if let Some(tx) = &progress_tx {
                tx.send(local_hashes.file_size_bytes()).await?;
            }
            return Ok(());
        }

        // Compare each chunk and download the differing chunks
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(output_file)?;
        let client = Client::new();
        let chunk_size = remote_hashes.chunk_size;
        for (i, (local_hash, remote_hash)) in local_hashes
            .hashes
            .iter()
            .zip(remote_hashes.hashes.iter())
            .enumerate()
        {
            if local_hash != remote_hash {
                let start = i * chunk_size;
                let end = start + chunk_size - 1;
                let range = format!("bytes={}-{}", start, end);

                let response = client
                    .get(file_url.to_string())
                    .header("Range", &range)
                    .send()
                    .await?;
                let chunk = response.bytes().await?;
                file.write_all_at(&chunk, start as u64)?;
                file.flush()?;

                if let Some(tx) = &progress_tx {
                    if !tx.is_closed() {
                        tx.send(end + 1).await?;
                    }
                }
            }
        }
        Ok(())
    }
}
