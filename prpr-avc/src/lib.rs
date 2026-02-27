mod avformat;
mod codec;
mod error;
mod ffi;
mod frame;
mod misc;
mod packet;
mod stream;
mod swr;
mod sws;
mod video;

pub use avformat::*;
pub use codec::*;
pub use error::*;
pub use frame::*;
pub use misc::*;
pub use packet::*;
pub use stream::*;
pub use swr::*;
pub use sws::*;
pub use video::*;

use sasa::{AudioClip, Frame};

const AUDIO_DECODING_SAMPLE_RATE: i32 = 44100;

pub fn demux_audio(file: impl AsRef<str>) -> Result<Option<AudioClip>> {
    demux_audio_with_options(file, false)
}

pub fn demux_audio_with_options(file: impl AsRef<str>, no_compress: bool) -> Result<Option<AudioClip>> {
    let mut format_ctx = AVFormatContext::new()?;
    format_ctx.open_input(file.as_ref())?;
    format_ctx.find_stream_info()?;

    let stream = match format_ctx.streams().into_iter().find(|it| it.is_audio()) {
        Some(stream) => stream,
        None => return Ok(None),
    };
    let decoder = stream.find_decoder()?;
    let mut codec_ctx = AVCodecContext::new(decoder, stream.codec_params(), None)?;

    let params = stream.codec_params();
    let in_format = AudioStreamFormat {
        channel_layout: params.channel_layout(),
        channels: params.channels(),
        sample_fmt: params.sample_format(),
        sample_rate: params.sample_rate(),
    };
    
    // 打印原始音频信息
    eprintln!("Original audio: sample_rate={}, channels={}, no_compress={}", 
              params.sample_rate(), params.channels(), no_compress);
    
    // 如果启用了不压缩选项，尽可能保留原始格式
    let (target_sample_rate, target_channels, target_channel_layout) = if no_compress {
        // 保留原始采样率和声道配置
        let channels = if params.channels() == 0 { 2 } else { params.channels() };
        let channel_layout = if params.channel_layout() == 0 {
            if channels == 1 { ffi::AV_CH_LAYOUT_MONO } else { ffi::AV_CH_LAYOUT_STEREO }
        } else {
            params.channel_layout()
        };
        eprintln!("No compress mode: keeping original sample_rate={}", params.sample_rate());
        (params.sample_rate(), channels, channel_layout)
    } else {
        eprintln!("Compress mode: resampling to {}", AUDIO_DECODING_SAMPLE_RATE);
        (AUDIO_DECODING_SAMPLE_RATE, 2, ffi::AV_CH_LAYOUT_STEREO)
    };
    
    let out_format = AudioStreamFormat {
        channel_layout: target_channel_layout,
        channels: target_channels,
        sample_fmt: ffi::AV_SAMPLE_FMT_FLT,
        sample_rate: target_sample_rate,
    };
    
    // 根据是否启用不压缩选项，选择不同质量的重采样器
    let mut swr = if no_compress {
        SwrContext::new_high_quality(&in_format, &out_format)?
    } else {
        SwrContext::new(&in_format, &out_format)?
    };
    swr.init()?;

    let mut in_frame = AVFrame::new()?;
    let mut frames = Vec::new();
    let mut packet = AVPacket::new()?;
    while format_ctx.read_frame(&mut packet)? {
        if packet.stream_index() == stream.index() {
            codec_ctx.send_packet(&packet)?;

            while codec_ctx.receive_frame(&mut in_frame)? {
                let end = frames.len();
                let out_samples = unsafe {
                    ffi::av_rescale_rnd(
                        swr.get_delay(in_format.sample_rate) + in_frame.number_of_samples() as i64,
                        target_sample_rate as _,
                        in_format.sample_rate as _,
                        ffi::AV_ROUND_UP,
                    )
                };

                frames.extend(std::iter::repeat_with(Frame::default).take(out_samples as usize));
                let out_samples = swr.convert(
                    in_frame.raw_data()[0],
                    in_frame.number_of_samples(),
                    unsafe { frames.as_mut_ptr().add(end) as *mut _ },
                    out_samples as _,
                )?;
                frames.truncate(end + out_samples);
            }
        }
    }

    Ok(Some(AudioClip::from_raw(frames, target_sample_rate as _)))
}
