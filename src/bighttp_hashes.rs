use anyhow::{anyhow, Result};
use bitcode::{Decode, Encode};
use blake3::Hasher;
use parking_lot::Mutex;
use rand::{thread_rng, Rng, RngCore};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use reqwest::{Client, StatusCode};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::Arc,
    usize,
};
use url::Url;

#[derive(Encode, Decode, Debug, PartialEq, PartialOrd)]
pub struct BigHTTPHashes<const HASH_SIZE: usize> {
    pub hashes: Vec<[u8; HASH_SIZE]>,
    pub chunk_size: usize,
    pub tail: usize,
}

impl<const HASH_SIZE: usize> BigHTTPHashes<HASH_SIZE> {
    pub fn noised(chunk_size: usize, file_size: usize) -> Self {
        let num_chunks = (file_size + chunk_size - 1) / chunk_size;
        let tail = file_size - (num_chunks - 1) * chunk_size;
        let hashes: Vec<[u8; HASH_SIZE]> = (0..num_chunks)
            .into_par_iter()
            .map(|_| {
                let mut result = [0u8; HASH_SIZE];
                thread_rng().fill_bytes(&mut result);
                result
            })
            .collect();
        Self {
            chunk_size,
            hashes,
            tail,
        }
    }

    pub async fn from_url(url: &Url) -> Result<Self> {
        let client = Client::new();

        let response = client.get(url.to_string()).send().await?;
        let resp_status = response.status();

        if resp_status == StatusCode::OK {
            let remote_hashes = response.bytes().await?;
            let remote_hashes: BigHTTPHashes<HASH_SIZE> = bitcode::decode(&remote_hashes)?;
            return Ok(remote_hashes);
        }

        return Err(anyhow!("HTTP Error: {}", resp_status));
    }

    pub fn from_file(file_path: &PathBuf, chunk_size: usize) -> Result<Self> {
        let file = Arc::new(Mutex::new(File::open(file_path)?));
        let file_size = file.lock().metadata()?.len();
        let num_chunks = (file_size + chunk_size as u64 - 1) / chunk_size as u64;
        let tail = (file_size - ((num_chunks - 1) * chunk_size as u64)) as usize;

        let hashes: Vec<[u8; HASH_SIZE]> = (0..num_chunks)
            .into_par_iter()
            .map(|chunk_index| {
                let mut buffer = vec![0; chunk_size];
                let offset = chunk_index * chunk_size as u64;
                let mut file = file.lock();
                file.seek(SeekFrom::Start(offset))?;
                let bytes_read = file.read(&mut buffer)?;
                drop(file);
                let mut hasher = Hasher::new();
                hasher.update(&buffer[..bytes_read]);
                let mut hash = [0; HASH_SIZE];
                let mut output_reader = hasher.finalize_xof();
                output_reader.fill(&mut hash);

                Ok(hash)
            })
            .collect::<Result<Vec<[u8; HASH_SIZE]>>>()?;

        Ok(Self {
            hashes,
            chunk_size,
            tail,
        })
    }

    pub fn file_size_bytes(&self) -> usize {
        (self.chunk_size * (self.hashes.len() - 1)) + self.tail
    }
}
