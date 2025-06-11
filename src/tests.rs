use crate::client::BigHttpClient;
use crate::hashes::BigHTTPHashes;
use anyhow::{anyhow, Result};
use hex_literal::hex;
use port_selector::random_free_port;
use rand::{Rng, RngCore, SeedableRng};
use reqwest::Client;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Read;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;
use std::time::Duration;
use tempfile::tempdir;
use tokio::time::sleep;
use url::Url;

fn append_random_bytes(file_path: &PathBuf) -> io::Result<()> {
    let mut rng = rand::thread_rng();
    // Generate a random number of bytes to append (e.g., between 1 and 1024)
    let num_bytes: usize = rng.gen_range(1..=1024);
    let random_bytes: Vec<u8> = (0..num_bytes).map(|_| rng.gen()).collect();

    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(file_path)?;

    file.write_all(&random_bytes)?;

    Ok(())
}

fn random_modify_bytes<R: Rng>(path: &PathBuf, rng: &mut R) -> Result<()> {
    // Read the file content into a Vec<u8>
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;

    // Randomly modify the bytes
    for byte in bytes.iter_mut() {
        if rng.gen_bool(0.1) {
            // 10% chance to modify each byte
            *byte = rng.gen();
        }
    }

    // Write the modified bytes back to the file
    let mut file = File::create(path)?;
    file.write_all(&bytes)?;

    Ok(())
}

async fn wait_for_http_200(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("http://localhost:{}", port);
    let client = Client::new();

    loop {
        match client.get(&url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    println!("Received HTTP 200 response from {}", url);
                    break;
                } else {
                    println!("Received non-200 response: {}", response.status());
                }
            }
            Err(e) => {
                println!("Request failed: {}", e);
            }
        }

        sleep(Duration::from_secs(1)).await;
    }

    Ok(())
}

fn get_last_n_bytes(data: &[u8], n: usize) -> &[u8] {
    let len = data.len();
    if len <= n {
        data
    } else {
        &data[len - n..]
    }
}

#[tokio::test]
async fn test_update_file() -> Result<()> {
    let mut seeded_rng = rand_chacha::ChaCha8Rng::from_seed(hex!(
        "558e94765820ad79881ca13af55630d492108f8274107c7df166cfede5e4110f"
    ));

    // Make a temporary file
    let temp_dir = tempdir()?;
    let file_path = temp_dir.path().join("test-file.bin");
    let hashes_path = temp_dir.path().join("test-file-hashes.bin");
    let file_path_out = temp_dir.path().join("test-file-out.bin");
    let mut file = File::create(&file_path)?;
    let mut bytes = vec![0u8; 1024];
    for _ in 0..1024 {
        seeded_rng.fill_bytes(&mut bytes);
        file.write_all(&bytes).unwrap();
    }
    let file_len = file.metadata()?.len();
    let hashes: BigHTTPHashes<8> = BigHTTPHashes::from_file(&file_path, 1024 * 1024)?;
    assert_eq!(hashes.file_size_bytes(), file_len as usize);
    let mut file = File::create(&hashes_path)?;
    file.write_all(&bitcode::encode(&hashes)).unwrap();

    println!(
        "File len: {}. Hashes len: {}",
        file_len,
        file.metadata()?.len()
    );

    let server_port = random_free_port().unwrap();
    let mut caddy_server = Command::new("caddy")
        .arg("file-server")
        .arg("--root")
        .arg(temp_dir.path().display().to_string())
        .arg("--browse")
        .arg("--listen")
        .arg(format!(":{}", server_port))
        .spawn()
        .unwrap();

    wait_for_http_200(server_port).await.unwrap();
    println!("Test server URL: http://localhost:{}", server_port);

    let mock_hashes_url = Url::from_str(&format!(
        "http://localhost:{}/test-file-hashes.bin",
        server_port
    ))
    .unwrap();
    let mock_file_url =
        Url::from_str(&format!("http://localhost:{}/test-file.bin", server_port)).unwrap();
    // Call the update_file method
    let big_http: BigHttpClient<8> = BigHttpClient::default();
    let remote_hashes: BigHTTPHashes<8> = BigHTTPHashes::from_url(&mock_hashes_url).await?;
    big_http
        .update_file(&remote_hashes, &mock_file_url, &file_path_out, None)
        .await?;

    let file1_content = std::fs::read(&file_path).unwrap();
    let file2_content = std::fs::read(&file_path_out).unwrap();

    assert_eq!(file1_content.len(), file2_content.len());

    let last_n_bytes1 = get_last_n_bytes(&file1_content, 64);
    let last_n_bytes2 = get_last_n_bytes(&file2_content, 64);

    // Verify the file content
    assert_eq!(hex::encode(last_n_bytes1), hex::encode(last_n_bytes2));
    assert!(file1_content == file2_content);

    //randomly modify the file to simulate an updated file
    random_modify_bytes(&file_path_out, &mut seeded_rng)
        .map_err(|e| anyhow!("Error corrupting {}: {}", file_path_out.display(), e))?;
    assert!(std::fs::read(&file_path).unwrap() != std::fs::read(&file_path_out).unwrap());

    // Call the update_file method
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    tokio::spawn(async move {
        loop {
            if let Some(size) = rx.recv().await {
                println!("Download update: {}", size);
            }
        }
    });
    big_http
        .update_file(&remote_hashes, &mock_file_url, &file_path_out, Some(tx))
        .await?;

    // Verify the file content
    let full_file_len = File::open(&file_path)?.metadata()?.len();
    assert_eq!(full_file_len, File::open(&file_path_out)?.metadata()?.len());
    assert!(std::fs::read(&file_path).unwrap() == std::fs::read(&file_path_out).unwrap());

    println!("Update after changing nothing!");
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    tokio::spawn(async move {
        while let Some(size_done) = rx.recv().await {
            assert_eq!(
                size_done, full_file_len as usize,
                "Should never show progress on a finished file!"
            );
        }
    });
    big_http
        .update_file(&remote_hashes, &mock_file_url, &file_path_out, Some(tx))
        .await?;

    // Verify the file content
    assert_eq!(
        File::open(&file_path)?.metadata()?.len(),
        File::open(&file_path_out)?.metadata()?.len()
    );
    assert!(std::fs::read(&file_path).unwrap() == std::fs::read(&file_path_out).unwrap());

    println!("Update after adding random bytes (Making original file bigger)");
    append_random_bytes(&file_path_out).unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    tokio::spawn(async move {
        loop {
            if let Some(size) = rx.recv().await {
                println!("Update after adding random bytes: {}", size);
            }
        }
    });
    big_http
        .update_file(&remote_hashes, &mock_file_url, &file_path_out, Some(tx))
        .await?;

    // Verify the file content
    assert_eq!(
        File::open(&file_path)?.metadata()?.len(),
        File::open(&file_path_out)?.metadata()?.len()
    );
    assert!(std::fs::read(&file_path).unwrap() == std::fs::read(&file_path_out).unwrap());

    temp_dir.close().unwrap();
    caddy_server.kill().unwrap();
    Ok(())
}
