use super::import::Import;
use crate::CatalogProducer;
use anyhow::{anyhow, Result};
use bytes::{Bytes, BytesMut};
use m3u8_rs::{AlternativeMediaType, MasterPlaylist, Playlist};
use moq_lite::BroadcastProducer;
use reqwest::Client;
use std::collections::{HashSet, VecDeque};
use std::time::Duration;
use url::Url;

const STARTUP_BUFFER_SEGMENTS: usize = 3;
const ROLLING_BUFFER_SEGMENTS: usize = 8;
const MAX_BUFFER_SEGMENTS: usize = ROLLING_BUFFER_SEGMENTS * 2;
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(10);

pub struct HlsIngest {
	importer: Import,
	playlist_url: Url,
	client: Client,
	current_map_uri: Option<String>,
	broadcast: BroadcastProducer,
}

impl HlsIngest {
	pub fn new(broadcast: BroadcastProducer, playlist_url: Url) -> Self {
		let broadcast_clone = broadcast.clone();
		let importer = Import::new(broadcast);
		let client = Client::builder()
			.connect_timeout(Duration::from_secs(5))
			.timeout(HTTP_TIMEOUT)
			.build()
			.expect("failed to build reqwest client");

		Self {
			importer,
			playlist_url,
			client,
			current_map_uri: None,
			broadcast: broadcast_clone,
		}
	}

	/// Start ingesting from a single playlist (muxed audio+video or single stream)
	pub async fn start(mut self) -> Result<()> {
		eprintln!("ingesting HLS from: {}", self.playlist_url);

		// First, check if the initial URL is a master playlist
		let bytes = self.fetch_with_retry(&self.playlist_url).await?;
		let bytes_slice = &bytes[..];

		match m3u8_rs::parse_playlist(bytes_slice) {
			Ok((_, Playlist::MasterPlaylist(master))) => {
				eprintln!(
					"initial URL is a master playlist with {} variants",
					master.variants.len()
				);

				// Log all variants for debugging
				for (i, variant) in master.variants.iter().enumerate() {
					let codecs = variant.codecs.as_ref().map(|c| c.as_str()).unwrap_or("unknown");
					eprintln!(
						"  variant {}: bandwidth={}, codecs={}, uri={}",
						i,
						variant.average_bandwidth.unwrap_or(variant.bandwidth),
						codecs,
						variant.uri
					);
				}

				let catalog = self.importer.catalog();
				return Box::pin(self.start_all_variants(catalog, master)).await;
			}
			Ok((_, Playlist::MediaPlaylist(_))) => {
				eprintln!("initial URL is a media playlist, processing single stream");
				// Continue with normal processing below
			}
			Err(e) => {
				return Err(anyhow::anyhow!("failed to parse initial playlist: {:?}", e));
			}
		}

		let mut next_media_sequence = 0;
		let mut buffering = true;
		let mut buffer = VecDeque::<Bytes>::new();

		loop {
			let bytes = self.fetch_with_retry(&self.playlist_url).await?;
			let bytes_slice = &bytes[..];

			match m3u8_rs::parse_playlist(bytes_slice) {
				Ok((_, Playlist::MediaPlaylist(pl))) => {
					let latest_map_uri = pl
						.segments
						.iter()
						.find_map(|segment| segment.map.as_ref().map(|map| map.uri.clone()));

					let latest_map_uri = latest_map_uri
						.ok_or_else(|| anyhow!("HLS CMAF playlist missing EXT-X-MAP; cannot initialize tracks"))?;

					if self.current_map_uri.as_ref() != Some(&latest_map_uri) {
						let init_uri = self.playlist_url.join(&latest_map_uri)?;
						eprintln!("downloading init Segment: {}", init_uri);
						let init_data = Self::strip_sidx(self.fetch_with_retry(&init_uri).await?);
						let mut init_slice: &[u8] = init_data.as_ref();
						self.importer.init_from(&mut init_slice).await?;
						self.current_map_uri = Some(latest_map_uri);
						eprintln!(
							"initialized CMAF tracks from latest EXT-X-MAP (init segment size: {} bytes)",
							init_data.len()
						);
						// Note: Audio and video tracks should be detected automatically by Import
						// if they are present in the init segment (muxed CMAF)
					}

					if next_media_sequence == 0 {
						// Start close to the live edge so we don't accumulate a huge backlog.
						next_media_sequence = pl
							.media_sequence
							.saturating_add(pl.segments.len().saturating_sub(2) as u64);
					}

					let start_seq = pl.media_sequence;
					for (i, segment) in pl.segments.iter().enumerate() {
						let current_seq = start_seq + i as u64;

						if current_seq >= next_media_sequence {
							let seg_url = self.playlist_url.join(&segment.uri)?;
							let seg_data = Self::strip_sidx(self.fetch_with_retry(&seg_url).await?);
							buffer.push_back(seg_data);
							eprintln!(
								"downloaded segment={} size={} buffered_frames={} buffering={}",
								current_seq,
								buffer.back().map(|b| b.len()).unwrap_or(0),
								buffer.len(),
								buffering
							);
							if buffering && buffer.len() >= STARTUP_BUFFER_SEGMENTS {
								buffering = false;
							}

							// Aggressively cap buffer to prevent memory buildup
							while buffer.len() > MAX_BUFFER_SEGMENTS {
								let dropped = buffer.pop_front();
								eprintln!(
                                    "dropping oldest buffered segment to cap size remaining={} (dropped segment size: {})",
                                    buffer.len(),
                                    dropped.as_ref().map(|b| b.len()).unwrap_or(0)
                                );
							}

							next_media_sequence = current_seq + 1;
						}
					}

					if !buffering {
						while buffer.len() > ROLLING_BUFFER_SEGMENTS {
							buffer.pop_front();
						}

						let max_segments = buffer.len().max(1).min(3);
						for _ in 0..max_segments {
							if let Some(buffered) = buffer.pop_front() {
								eprintln!("parsing buffered segment remaining={}", buffer.len());
								self.importer.parse(&buffered)?;
							} else {
								break;
							}
						}
					}

					let sleep_time = if buffering {
						Duration::from_millis(150)
					} else if buffer.is_empty() {
						Duration::from_millis(40)
					} else {
						Duration::from_millis(80)
					};
					tokio::time::sleep(sleep_time).await;
				}
				Ok((_, Playlist::MasterPlaylist(master))) => {
					eprintln!("master playlist detected with {} variants", master.variants.len());

					// Log all variants for debugging
					for (i, variant) in master.variants.iter().enumerate() {
						let codecs = variant.codecs.as_ref().map(|c| c.as_str()).unwrap_or("unknown");
						eprintln!(
							"  variant {}: bandwidth={}, codecs={}, uri={}",
							i,
							variant.average_bandwidth.unwrap_or(variant.bandwidth),
							codecs,
							variant.uri
						);
					}

					// Process all variants - each will add its tracks to the shared broadcast
					// This allows audio and video (and other tracks) to be processed separately
					// Each variant calls init_from with its own init segment (as discussed with Luke)
					let catalog = self.importer.catalog();
					// Use Box::pin to avoid recursion issues
					return Box::pin(self.start_all_variants(catalog, master)).await;
				}
				Err(e) => {
					eprintln!("error parsing  {:?}", e);
					tokio::time::sleep(Duration::from_secs(1)).await;
				}
			}
		}
	}
}

impl HlsIngest {
	/// Start ingesting all variants from a master playlist in parallel
	/// Each variant will process its own playlist and add tracks to the shared broadcast
	/// Each variant calls init_from with its own init segment (as discussed with Luke)
	async fn start_all_variants(self, catalog: CatalogProducer, master: MasterPlaylist) -> Result<()> {
		let HlsIngest {
			importer: _,
			playlist_url: master_playlist_url,
			client,
			current_map_uri: _,
			broadcast,
		} = self;

		let MasterPlaylist {
			variants, alternatives, ..
		} = master;

		// Filter out I-frame only variants
		let media_variants: Vec<_> = variants.into_iter().filter(|v| !v.is_i_frame).collect();

		if media_variants.is_empty() {
			return Err(anyhow::anyhow!("master playlist does not contain any media variants"));
		}

		eprintln!("processing {} variants in parallel", media_variants.len());

		// Process all variants concurrently by interleaving their processing
		// Each variant will add its tracks to the shared broadcast
		// Each variant calls init_from with its own init segment (as discussed with Luke)
		eprintln!(
			"processing {} variants concurrently (each will add tracks to shared broadcast)",
			media_variants.len()
		);

		// Build ingest states for each variant using a shared catalog.
		let mut variant_states: Vec<VariantState> = Vec::new();
		let mut next_index = 0usize;

		for (i, variant) in media_variants.iter().enumerate() {
			let variant_url = master_playlist_url.join(&variant.uri)?;
			let codecs = variant.codecs.as_ref().map(|c| c.as_str()).unwrap_or("unknown");
			eprintln!("  preparing variant {}: {} (codecs: {})", i, variant_url, codecs);

			variant_states.push(VariantState {
				index: next_index,
				importer: Import::with_catalog(broadcast.clone(), catalog.clone()),
				playlist_url: variant_url,
				client: client.clone(),
				current_map_uri: None,
				next_media_sequence: 0,
				buffering: true,
				buffer: VecDeque::new(),
			});
			next_index += 1;
		}

		if variant_states.is_empty() {
			return Err(anyhow::anyhow!("no variants to process"));
		}

		let mut seen_audio = HashSet::new();
		for variant in &media_variants {
			if let Some(group_id) = &variant.audio {
				for alt in alternatives
					.iter()
					.filter(|alt| alt.media_type == AlternativeMediaType::Audio && &alt.group_id == group_id)
				{
					if let Some(uri) = &alt.uri {
						let key = format!("{}::{}", group_id, uri);
						if seen_audio.insert(key) {
							let audio_url = master_playlist_url.join(uri)?;
							eprintln!("  preparing audio group {} ({}) -> {}", group_id, alt.name, audio_url);
							variant_states.push(VariantState {
								index: next_index,
								importer: Import::with_catalog(broadcast.clone(), catalog.clone()),
								playlist_url: audio_url,
								client: client.clone(),
								current_map_uri: None,
								next_media_sequence: 0,
								buffering: true,
								buffer: VecDeque::new(),
							});
							next_index += 1;
						}
					}
				}
			}
		}

		// Process all variants in an interleaved manner
		// Each variant will have its own Import instance and call init_from separately
		// All will write to the same broadcast
		eprintln!("starting all variants in interleaved processing mode");
		eprintln!("NOTE: Each variant will initialize its tracks independently to the same broadcast");

		// Process all variants in a round-robin fashion
		loop {
			let mut any_active = false;

			for state in &mut variant_states {
				match Self::process_variant_iteration(state).await {
					Ok(true) => {
						any_active = true;
					}
					Ok(false) => {
						// This variant is done, but continue with others
					}
					Err(e) => {
						eprintln!("[variant {}] error: {}, continuing with other variants", state.index, e);
						// Continue with other variants
						any_active = true;
					}
				}

				// Yield to allow other tasks
				tokio::task::yield_now().await;
			}

			if !any_active {
				eprintln!("all variants completed");
				break;
			}

			// Small sleep to prevent busy-waiting
			tokio::time::sleep(Duration::from_millis(10)).await;
		}

		Ok(())
	}

	/// Process one iteration of a variant's ingest loop
	/// Returns true if should continue, false if done
	async fn process_variant_iteration(state: &mut VariantState) -> Result<bool> {
		// Fetch playlist
		let bytes = Self::fetch_with_retry_internal(&state.client, &state.playlist_url).await?;
		let bytes_slice = &bytes[..];

		match m3u8_rs::parse_playlist(bytes_slice) {
			Ok((_, Playlist::MediaPlaylist(pl))) => {
				// Handle init segment
				let latest_map_uri = pl
					.segments
					.iter()
					.find_map(|segment| segment.map.as_ref().map(|map| map.uri.clone()));

				if let Some(map_uri) = latest_map_uri {
					if state.current_map_uri.as_ref() != Some(&map_uri) {
						let init_uri = state.playlist_url.join(&map_uri)?;
						eprintln!("[variant {}] downloading init Segment: {}", state.index, init_uri);
						let init_data =
							Self::strip_sidx(Self::fetch_with_retry_internal(&state.client, &init_uri).await?);
						let mut init_slice: &[u8] = init_data.as_ref();
						state.importer.init_from(&mut init_slice).await?;
						state.current_map_uri = Some(map_uri);
						eprintln!(
							"[variant {}] initialized CMAF tracks (init segment size: {} bytes)",
							state.index,
							init_data.len()
						);
					}
				} else {
					return Err(anyhow::anyhow!("HLS CMAF playlist missing EXT-X-MAP"));
				}

				// Download new segments
				if state.next_media_sequence == 0 {
					state.next_media_sequence = pl
						.media_sequence
						.saturating_add(pl.segments.len().saturating_sub(2) as u64);
				}

				let start_seq = pl.media_sequence;
				for (i, segment) in pl.segments.iter().enumerate() {
					let current_seq = start_seq + i as u64;

					if current_seq >= state.next_media_sequence {
						let seg_url = state.playlist_url.join(&segment.uri)?;
						let seg_data =
							Self::strip_sidx(Self::fetch_with_retry_internal(&state.client, &seg_url).await?);
						state.buffer.push_back(seg_data);

						if state.buffering && state.buffer.len() >= STARTUP_BUFFER_SEGMENTS {
							state.buffering = false;
						}

						// Cap buffer
						while state.buffer.len() > MAX_BUFFER_SEGMENTS {
							state.buffer.pop_front();
						}

						state.next_media_sequence = current_seq + 1;
					}
				}

				if !state.buffering {
					while state.buffer.len() > ROLLING_BUFFER_SEGMENTS {
						state.buffer.pop_front();
					}

					let max_segments = state.buffer.len().max(1).min(3);
					for _ in 0..max_segments {
						if let Some(buffered) = state.buffer.pop_front() {
							state.importer.parse(&buffered)?;
						} else {
							break;
						}
					}
				}

				let sleep_time = if state.buffering {
					Duration::from_millis(150)
				} else if state.buffer.is_empty() {
					Duration::from_millis(40)
				} else {
					Duration::from_millis(80)
				};
				tokio::time::sleep(sleep_time).await;

				Ok(true) // Continue processing
			}
			Ok((_, Playlist::MasterPlaylist(_))) => {
				eprintln!("[variant {}] unexpected master playlist in variant", state.index);
				Ok(false)
			}
			Err(e) => {
				eprintln!("[variant {}] error parsing playlist: {:?}", state.index, e);
				tokio::time::sleep(Duration::from_secs(1)).await;
				Ok(true) // Continue despite error
			}
		}
	}

	async fn fetch_with_retry_internal(client: &Client, url: &Url) -> Result<Bytes> {
		let mut delay = Duration::from_secs(1);
		loop {
			match client.get(url.clone()).send().await {
				Ok(resp) => match resp.error_for_status() {
					Ok(success) => match success.bytes().await {
						Ok(bytes) => return Ok(bytes),
						Err(err) => eprintln!("error reading body {}: {}", url, err),
					},
					Err(status_err) => eprintln!("http error {}: {}", url, status_err),
				},
				Err(err) => eprintln!("request error {}: {}", url, err),
			}

			eprintln!("retrying fetch {} after {:?}", url, delay);
			tokio::time::sleep(delay).await;
			delay = (delay * 2).min(MAX_RETRY_DELAY);
		}
	}

	async fn fetch_with_retry(&self, url: &Url) -> Result<Bytes> {
		Self::fetch_with_retry_internal(&self.client, url).await
	}

	/// Remove optional sidx boxes from CMAF segments so the importer doesn't log warnings.
	fn strip_sidx(bytes: Bytes) -> Bytes {
		let data = bytes.as_ref();
		let mut pos = 0;
		let len = data.len();
		let mut sanitized: Option<BytesMut> = None;

		while pos + 8 <= len {
			let size = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
			let mut box_len = size as usize;

			if box_len == 0 {
				box_len = len - pos;
			} else if box_len == 1 {
				if pos + 16 > len {
					break;
				}
				let mut extended = [0u8; 8];
				extended.copy_from_slice(&data[pos + 8..pos + 16]);
				box_len = u64::from_be_bytes(extended) as usize;
			}

			if box_len == 0 || pos + box_len > len {
				break;
			}

			let kind = &data[pos + 4..pos + 8];

			if kind == b"sidx" {
				sanitized.get_or_insert_with(|| {
					let mut buf = BytesMut::with_capacity(len.saturating_sub(box_len));
					buf.extend_from_slice(&data[..pos]);
					buf
				});
			} else if let Some(buf) = sanitized.as_mut() {
				buf.extend_from_slice(&data[pos..pos + box_len]);
			}

			pos += box_len;
		}

		if let Some(mut buf) = sanitized {
			if pos < len {
				buf.extend_from_slice(&data[pos..]);
			}
			buf.freeze()
		} else {
			bytes
		}
	}
}

// State for processing a single variant
struct VariantState {
	index: usize,
	importer: Import,
	playlist_url: Url,
	client: Client,
	current_map_uri: Option<String>,
	next_media_sequence: u64,
	buffering: bool,
	buffer: VecDeque<Bytes>,
}
