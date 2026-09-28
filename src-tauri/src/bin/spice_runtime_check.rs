use spice_client::SpiceClientShared;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = SpiceClientShared::new("127.0.0.1".into(), 5556);
    client.connect().await?;
    client.start_event_loop().await?;
    sleep(Duration::from_secs(2)).await;
    let surface = client.get_display_surface(0).await.ok_or("no surface")?;
    println!(
        "nonzero={}",
        surface.data.iter().filter(|byte| **byte != 0).count()
    );
    for offset in [0, 10, 20] {
        match client
            .send_mouse_position(0, surface.width / 2 + offset, surface.height / 2, 0)
            .await
        {
            Ok(()) => println!("absolute_pointer_{offset}=write_ok"),
            Err(error) => println!("absolute_pointer_{offset}=write_failed: {error}"),
        }
        sleep(Duration::from_millis(250)).await;
    }
    client.disconnect().await;
    Ok(())
}
