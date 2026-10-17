// 实验验证：本测试针对临时后端，和最终成品 API 可能相差很大。
#![forbid(unsafe_code)]
#[path = "../host/src/backend.rs"]
mod backend;

#[cfg(test)]
mod tests {
    use super::backend::*;
    use std::{path::PathBuf, process::Command, thread, time::{Duration, Instant}};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("vestra-experimental1-test-{}.mp4", std::process::id()));
            let status = Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-y",
                "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30",
                "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000",
                "-t", "4", "-c:v", "mpeg4", "-q:v", "3", "-c:a", "aac"])
                .arg(&path).status().unwrap();
            assert!(status.success());
            Self(path)
        }
    }
    impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); } }

    fn frame(backend: &mut Backend) -> Frame {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if let Some(frame) = backend.poll().unwrap() { return frame; }
            assert!(Instant::now() < deadline, "视频帧超时");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn actual_mp4_decode_pause_seek_gaussian_and_stop() {
        let fixture = Fixture::new();
        let mut backend = Backend::new_silent().unwrap();
        backend.load(fixture.0.clone()).unwrap();
        assert!((backend.snapshot().duration - 4.0).abs() < 0.05);
        let initial = frame(&mut backend);
        assert_eq!(initial.rgb.len(), (WIDTH * HEIGHT * 3) as usize);
        assert!(initial.rgb.iter().any(|v| *v > 0));
        backend.toggle_pause().unwrap();
        assert!(backend.snapshot().paused);
        frame(&mut backend);
        let paused_position = backend.snapshot().position;
        thread::sleep(Duration::from_millis(200));
        assert!(backend.poll().unwrap().is_none());
        assert_eq!(backend.snapshot().position, paused_position);

        backend.seek(1.0).unwrap();
        let clear = frame(&mut backend);
        assert!((clear.position - 1.0).abs() < 0.001);
        assert!(backend.snapshot().paused);
        backend.set_gaussian(true, 5.0).unwrap();
        let blur = frame(&mut backend);
        assert!(backend.snapshot().paused);
        assert_eq!(clear.position, blur.position);
        let difference: u64 = clear.rgb.iter().zip(&blur.rgb)
            .map(|(a, b)| a.abs_diff(*b) as u64).sum();
        assert!(difference > 100_000, "gblur 应当改变真实帧，差值={difference}");
        assert!(backend.set_gaussian(true, f64::NAN).is_err());
        assert!(backend.set_gaussian(true, 13.0).is_err());
        assert!(backend.seek(f64::INFINITY).is_err());

        backend.toggle_pause().unwrap();
        frame(&mut backend);
        thread::sleep(Duration::from_millis(150));
        let later = frame(&mut backend);
        assert!(later.position > 1.0);
        backend.stop();
        assert!(backend.poll().unwrap().is_none(), "stop 必须丢弃旧进程帧");
        assert!(backend.snapshot().paused);
        drop(backend);
        println!("真实 MP4 解码、暂停、seek、暂停预览、高斯滤镜、参数验证、退出清理通过；像素差值={difference}");
    }
}
