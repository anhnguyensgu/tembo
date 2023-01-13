use kube::{Client, ResourceExt};
use log::info;
use pgmq::PGMQueue;
use reconciler::{
    create_ing_route_tcp, create_namespace, create_or_update, delete, delete_namespace,
    generate_spec, get_all,
};
use std::env;
use std::{thread, time};

#[tokio::main]
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    // // Read connection info from environment variable
    let pg_conn_url = env::var("PG_CONN_URL").expect("PG_CONN_URL must be set");
    let pg_queue_name = env::var("PG_QUEUE_NAME").expect("PG_QUEUE_NAME must be set");

    // Connect to postgres queue
    let queue: PGMQueue = PGMQueue::new(pg_conn_url).await;

    // Infer the runtime environment and try to create a Kubernetes Client
    let client = Client::try_default().await?;

    loop {
        // Read from queue (check for new message)
        let read_msg = match queue.read(&pg_queue_name, Some(&30_u32)).await {
            Some(message) => {
                info!("read_msg: {:?}", message);
                message
            }
            None => {
                thread::sleep(time::Duration::from_secs(1));
                continue;
            }
        };

        // Based on message_type in message, create, update, delete PostgresCluster
        match serde_json::from_str(&read_msg.message["message_type"].to_string()).unwrap() {
            Some("SnapShot") => {
                info!("Doing nothing for now")
            }
            Some("Create") | Some("Update") => {
                // create namespace if it does not exist
                let namespace: String =
                    serde_json::from_value(read_msg.message["body"]["resource_name"].clone())
                        .unwrap();
                create_namespace(client.clone(), namespace.clone())
                    .await
                    .expect("error creating namespace");

                // create IngressRouteTCP
                create_ing_route_tcp(client.clone(), namespace.clone())
                    .await
                    .expect("error creating IngressRouteTCP");

                // generate PostgresCluster spec based on values in body
                let spec = generate_spec(read_msg.message["body"].clone()).await;

                // create or update PostgresCluster
                create_or_update(client.clone(), namespace.clone(), spec)
                    .await
                    .expect("error creating or updating PostgresCluster");
            }
            Some("Delete") => {
                let name: String =
                    serde_json::from_value(read_msg.message["body"]["resource_name"].clone())
                        .unwrap();

                // delete PostgresCluster
                delete(client.clone(), name.clone(), name.clone())
                    .await
                    .expect("error deleting PostgresCluster");

                // delete namespace
                delete_namespace(client.clone(), name.clone())
                    .await
                    .expect("error deleting namespace");
            }
            None | _ => info!("action was not in expected format"),
        }

        // TODO(ianstanton) This is here as an example for now. We want to use
        //  this to ensure a PostgresCluster exists before we attempt to delete it.
        // Get all existing PostgresClusters
        let vec = get_all(client.clone(), "default".to_owned());
        for pg in vec.await.iter() {
            info!("found PostgresCluster {}", pg.name_any());
        }
        thread::sleep(time::Duration::from_secs(1));

        // Delete message from queue
        let deleted = queue
            .delete(&pg_queue_name, &read_msg.msg_id)
            .await
            .expect("error deleting message from queue");
        // TODO(ianstanton) Improve logging everywhere
        info!("deleted: {:?}", deleted);
    }
}

fn main() {
    env_logger::init();
    info!("starting");
    run().unwrap();
}
