use std::{collections::VecDeque, sync::mpsc};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, Host, SampleRate, Stream, SupportedStreamConfig,
};
use eframe::{egui, App};

pub struct Application {
    host: Host,
    output_device: Device,
    output_device_config: SupportedStreamConfig,
    output_stream: Stream,
    output_sender: mpsc::SyncSender<Vec<f32>>,
    output_rem_receiver: mpsc::Receiver<usize>,
    input_stream: Stream,
    input_receiver: mpsc::Receiver<Vec<f32>>,
    input_sender: mpsc::SyncSender<bool>,

    buffers: Vec<Vec<f32>>,
    buffer: Vec<f32>,
    encoded_buffer: Vec<u8>,

    recording: bool,

    rem: usize,
    frequency: f64,
    offset: usize,
    length: usize,
    speed: f32,

    resolution: usize,
}

impl Default for Application {
    fn default() -> Self {
        let host = cpal::default_host();

        let output_device = host.default_output_device().unwrap();
        let output_device_config = output_device.default_output_config().unwrap();

        let input_device = host.default_input_device().unwrap();

        let (output_sender, output_receiver) = mpsc::sync_channel::<Vec<f32>>(16);
        let (output_rem_sender, output_rem_receiver) = mpsc::sync_channel::<usize>(16);
        let (input_sender, input_receiver) = mpsc::sync_channel::<Vec<f32>>(16);
        let (input_sender_rec, input_receiver_rec) = mpsc::sync_channel(16);

        let output_stream = {
            let mut buffer = VecDeque::new();
            output_device
                .build_output_stream(
                    &cpal::StreamConfig {
                        channels: 1,
                        sample_rate: SampleRate(48000),
                        buffer_size: cpal::BufferSize::Default,
                    },
                    move |data: &mut [f32], _| {
                        while let Ok(buff) = output_receiver.try_recv() {
                            buffer.extend(buff);
                        }
                        for byte in data.iter_mut() {
                            *byte = if let Some(b) = buffer.pop_front() {
                                b
                            } else {
                                0.0
                            }
                        }
                        let _ = output_rem_sender.send(buffer.len());
                    },
                    |error| eprintln!("Output stream Error: {error}"),
                    None,
                )
                .unwrap()
        };

        let input_stream = {
            let mut recording = false;
            input_device
                .build_input_stream(
                    &cpal::StreamConfig {
                        channels: 1,
                        sample_rate: SampleRate(48000),
                        buffer_size: cpal::BufferSize::Default,
                    },
                    move |data: &[f32], _| {
                        while let Ok(rec) = input_receiver_rec.try_recv() {
                            recording = rec;
                        }
                        if recording {
                            let _ = input_sender.send(data.to_vec());
                        }
                    },
                    |error| eprintln!("Input stream Error: {error}"),
                    None,
                )
                .unwrap()
        };

        Self {
            host,
            output_device,
            output_device_config,
            output_stream,
            output_sender,
            output_rem_receiver,
            rem: 0,
            offset: 0,
            length: 48 * 20,
            buffer: vec![],
            speed: 1.0,
            frequency: 0.1,
            recording: false,
            input_stream,
            input_receiver,
            input_sender: input_sender_rec,
            resolution: 1000,
            encoded_buffer: vec![],
            buffers: vec![],
        }
    }
}

impl App for Application {
    fn update(&mut self, ctx: &eframe::egui::Context, frame: &mut eframe::Frame) {
        while let Ok(rem) = self.output_rem_receiver.try_recv() {
            self.rem = rem;
        }
        while let Ok(buff) = self.input_receiver.try_recv() {
            if self.recording {
                self.buffer.extend(buff)
            }
        }
        egui::TopBottomPanel::bottom("Controls").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::widgets::DragValue::new(&mut self.length).prefix("Length: "));
                ui.add(egui::DragValue::new(&mut self.offset).prefix("Offset: "));
                ui.add(egui::DragValue::new(&mut self.speed).prefix("Speed: "));
                ui.add(
                    egui::DragValue::new(&mut self.frequency)
                        .prefix("Freq: ")
                        .speed(0.00001)
                        .clamp_range(0..=1),
                );
            });

            if ui.checkbox(&mut self.recording, "Recording").changed() {
                let _ = self.input_sender.send(self.recording);
            }

            if ui.button("Sin").clicked() {
                self.buffer.resize(self.buffer.len().max(self.length), 0.0);
                for i in 0..self.length {
                    let sample = i as f32;
                    let sample = (sample as f64 * self.frequency).sin() as f32 * 0.5;
                    self.buffer[i] += sample;
                }
            }

            if ui.button("Encode").clicked() {
                let mut new_buffer = Vec::new();
                let mut last = 0.0;
                for byte in self.buffer.iter() {
                    let byte1 = *byte as f64 * i16::MAX as f64;
                    let byte1 = (byte1 - (last as f64 * i16::MAX as f64)) as i16;
                    last = *byte;
                    if byte1 < i8::MAX as i16 && (i8::MIN as i16) < byte1 {
                        println!("pbyte: {byte1}");
                        let mut byte1 = byte1 as u8;
                        if byte1 & 1 == 1 {
                            byte1 -= 1;
                        }
                        new_buffer.push(byte1)
                    } else {
                        println!("nbyte: {byte1}");
                        let mut bytes = byte1.to_le_bytes();
                        if bytes[0] & 1 == 0 {
                            bytes[0] += 1;
                        }
                        new_buffer.extend(bytes);
                    }
                }
                println!("Encoded: size {}", new_buffer.len());
                self.encoded_buffer = new_buffer;
            }

            if ui.button("Decode").clicked() {
                let mut new_buffer = Vec::new();
                let mut last = 0f32;
                let mut encoded_buffer = self.encoded_buffer.clone();
                let mut iter = encoded_buffer.drain(..);
                while let Some(byte) = iter.next() {
                    println!("Byte: {byte:8b}");
                    if byte & 1 == 0 {
                        if let Some(seccond_byte) = iter.next() {
                            let byte =
                                i16::from_le_bytes([byte, seccond_byte]) as f64 / i16::MAX as f64;
                            let byte = byte + last as f64;
                            let byte = byte as f32;
                            last = byte;
                            new_buffer.push(byte);
                        }
                    } else {
                        let byte = (byte as i8) as f64 / i16::MAX as f64;
                        let byte = byte + last as f64;
                        let byte = byte as f32;
                        last = byte;
                        new_buffer.push(byte);
                    }
                }
                self.buffer = new_buffer;
                println!(
                    "Decoded: size {}",
                    self.buffer.len() * std::mem::size_of::<f32>()
                );
            }

            if ui.button("Clear").clicked() {
                self.buffer.clear();
            }

            if ui.button("Process").clicked() {
                let len = (self.buffer.len() as f32 / self.speed).round() as usize;
                let mut new_buffer = Vec::with_capacity(len);
                for i in 0..len {
                    let original_index = (i as f32 * self.speed).round();
                    let left_index = original_index.floor() as usize;
                    let right_index = left_index + 1;
                    let fractional = original_index.fract();

                    if right_index < self.buffer.len() {
                        let left_sample = self.buffer[left_index];
                        let right_sample = self.buffer[right_index];

                        let interpolate_sample =
                            (1.0 - fractional) * left_sample + fractional * right_sample;
                        new_buffer.push(interpolate_sample);
                    } else if left_index < self.buffer.len() {
                        new_buffer.push(self.buffer[left_index])
                    }
                }
                self.buffer = new_buffer;
            }

            if ui.button("Submit").clicked() {
                let _ = self.output_sender.send(self.buffer.clone());
                self.rem += 1;
            }

            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    self.buffers.push(std::mem::take(&mut self.buffer));
                }
                for i in 0..self.buffers.len() {
                    if ui.button(format!("Load: {i}")).clicked() {
                        self.buffer = self.buffers[i].clone();
                    }
                }
            });

            ui.spinner();
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add(
                egui::DragValue::new(&mut self.resolution)
                    .prefix("Resolution: ")
                    .speed(1),
            );
            egui::plot::Plot::new("Buffer").show(ui, |ui| {
                let bounds = ui.plot_bounds();
                let range = bounds.min()[0].max(0.0)..bounds.max()[0].min(self.buffer.len() as f64);
                ui.line(egui::plot::Line::new(
                    egui::plot::PlotPoints::from_parametric_callback(
                        |t| {
                            let i = t as usize;
                            if self.buffer.len() > i {
                                (t, self.buffer[i] as f64)
                            } else {
                                (0.0, 0.0)
                            }
                        },
                        range,
                        self.resolution.min(self.buffer.len()),
                    ),
                ));
                ui.vline(
                    egui::plot::VLine::new(self.buffer.len() as f32 - self.rem as f32).name("Rem"),
                )
            });
        });
    }
}

impl Drop for Application {
    fn drop(&mut self) {
        let _ = self.input_stream.pause();
        let _ = self.output_stream.pause();
        self.buffer.clear();
        self.buffers.clear();
        self.encoded_buffer.clear();
    }
}

fn main() {
    eframe::run_native(
        "codec-map",
        eframe::NativeOptions::default(),
        Box::new(|_| Box::<Application>::default()),
    )
    .unwrap();
}
