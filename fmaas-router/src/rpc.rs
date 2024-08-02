pub mod generation;
pub mod nlp;
pub mod info;

use tonic::{Code, Request, Status};


const METADATA_NAME_MODEL_ID: &str = "mm-model-id";

/// Extracts model_id from [`Request`] metadata.
fn extract_model_id<T>(request: &Request<T>) -> Result<&str, Status> {
    let metadata = request.metadata();
    if !metadata.contains_key(METADATA_NAME_MODEL_ID) {
        return Err(Status::new(
            Code::InvalidArgument,
            "Missing required model ID",
        ));
    }
    let model_id = metadata
        .get(METADATA_NAME_MODEL_ID)
        .unwrap()
        .to_str()
        .unwrap();
    Ok(model_id)
}