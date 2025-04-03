#![feature(coroutines)]
#![feature(coroutine_trait)]
use std::fs::{OpenOptions, File};
use std::io::{Write, self};
use std::time::Instant;
use rand::Rng;

use std::ops::{Coroutine, CoroutineState};
use std::pin::Pin;
use std::io::{BufRead, BufReader};


struct WriteCoroutine {
    pub file_handle: File,
}

impl WriteCoroutine {
    fn new(path: &str) -> io::Result<Self> {
        let file_handle = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self { file_handle })
    }
}

impl Coroutine<i32> for WriteCoroutine {
    type Yield = ();
    type Return = ();

    fn resume(mut self: Pin<&mut Self>, arg: i32)
              -> CoroutineState<Self::Yield, Self::Return> {
        writeln!(self.file_handle, "{}", arg).unwrap();
        CoroutineState::Yielded(())
    }
}

struct ReadCoroutine {
    lines: io::Lines<BufReader<File>>,
}

impl ReadCoroutine {
    fn new(path: &str) -> io::Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let lines = reader.lines();

        Ok(Self { lines })
    }
}

impl Coroutine<()> for ReadCoroutine {
    type Yield = i32;
    type Return = ();

    fn resume(mut self: Pin<&mut Self>, _arg: ())
              -> CoroutineState<Self::Yield, Self::Return> {
        match self.lines.next() {
            Some(Ok(line)) => {
                if let Ok(number) = line.parse::<i32>() {
                    CoroutineState::Yielded(number)
                } else {
                    CoroutineState::Complete(())
                }
            }
            Some(Err(_)) | None => CoroutineState::Complete(()),
        }
    }
}

struct CoroutineManager{
    reader: ReadCoroutine,
    writer: WriteCoroutine
}

impl CoroutineManager {
    fn new(read_path: &str, write_path: &str) -> io::Result<Self> {
        let reader = ReadCoroutine::new(read_path)?;
        let writer = WriteCoroutine::new(write_path)?;

        Ok(Self {
            reader,
            writer,
        })
    }
    fn run(&mut self) {
        let mut read_pin = Pin::new(&mut self.reader);
        let mut write_pin = Pin::new(&mut self.writer);

        loop {
            match read_pin.as_mut().resume(()) {
                CoroutineState::Yielded(number) => {
                    write_pin.as_mut().resume(number);
                }
                CoroutineState::Complete(()) => break,
            }
        }
    }
}


fn append_number_to_file(n: i32) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("numbers.txt")?;
    writeln!(file, "{}", n)?;
    Ok(())
}

trait SymmetricCoroutine {
    type Input;
    type Output;

    fn resume_with_input(
        self: Pin<&mut Self>, input: Self::Input
    ) -> Self::Output;
}

impl SymmetricCoroutine for ReadCoroutine {
    type Input = ();
    type Output = Option<i32>;

    fn resume_with_input(
        mut self: Pin<&mut Self>, _input: ()
    ) -> Self::Output {
        if let Some(Ok(line)) = self.lines.next() {
            line.parse::<i32>().ok()
        } else {
            None
        }
    }
}

impl SymmetricCoroutine for WriteCoroutine {
    type Input = i32;
    type Output = ();

    fn resume_with_input(
        mut self: Pin<&mut Self>, input: i32
    ) -> Self::Output {
        writeln!(self.file_handle, "{}", input).unwrap();
    }
}


fn main() -> io::Result<()> {
    let mut rng = rand::thread_rng();
    let numbers: Vec<i32> = (0..200000).map(|_| rng.r#gen()).collect();

    let start = Instant::now();
    let mut coroutine = WriteCoroutine::new(
        "./data/numbers.txt"
    )?;
    for &number in &numbers {
        Pin::new(&mut coroutine).resume(number);
    }
    // for &number in &numbers {
    //     if let Err(e) = append_number_to_file(number) {
    //         eprintln!("Failed to write to file: {}", e);
    //     }
    // }
    let duration = start.elapsed();

    println!("Time elapsed in file operations is: {:?}", duration);


    let mut coroutine = ReadCoroutine::new("./data/data.txt")?;

    loop {
        match Pin::new(&mut coroutine).resume(()) {
            CoroutineState::Yielded(number) => println!("{:?}", number),
            CoroutineState::Complete(()) => break,
        }
    }

    // let mut manager = CoroutineManager::new(
    //     "./data/numbers.txt", "./data/output.txt"
    // ).unwrap();
    // manager.run();

    let mut reader = ReadCoroutine::new("./data/numbers.txt")?;
    let mut writer = WriteCoroutine::new("./data/output.txt")?;

    loop {
        let number = Pin::new(&mut reader).resume_with_input(());
        if let Some(num) = number {
            Pin::new(&mut writer).resume_with_input(num);
        } else {
            break;
        }
    }
    Ok(())
}

