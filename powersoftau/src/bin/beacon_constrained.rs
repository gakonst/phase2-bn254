use powersoftau::bn256::Bn256CeremonyParameters;

use powersoftau::batched_accumulator::BatchedAccumulator;
use powersoftau::keypair::keypair;
use powersoftau::parameters::{CheckForCorrectness, UseCompression};

use bellman_ce::pairing::bn256::Bn256;
use memmap::*;
use std::fs::OpenOptions;

use std::io::Write;

use powersoftau::parameters::PowersOfTauParameters;

#[macro_use]
extern crate hex_literal;

const INPUT_IS_COMPRESSED: UseCompression = UseCompression::No;
const COMPRESS_THE_OUTPUT: UseCompression = UseCompression::Yes;
const CHECK_INPUT_CORRECTNESS: CheckForCorrectness = CheckForCorrectness::No;

#[allow(clippy::modulo_one)]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        println!("Usage: \n<challenge_file> <response_file>");
        std::process::exit(exitcode::USAGE);
    }
    let challenge_filename = &args[1];
    let response_filename = &args[2];

    println!(
        "Will contribute a random beacon to accumulator for 2^{} powers of tau",
        Bn256CeremonyParameters::REQUIRED_POWER
    );
    println!(
        "In total will generate up to {} powers",
        Bn256CeremonyParameters::TAU_POWERS_G1_LENGTH
    );

    // Create an RNG based on the outcome of the random beacon
    let mut rng = {
        use byteorder::{BigEndian, ReadBytesExt};
        use crypto::digest::Digest;
        use crypto::sha2::Sha256;
        use rand::chacha::ChaChaRng;
        use rand::SeedableRng;

        // Place block hash here (block number #564321)
        let mut cur_hash: [u8; 32] =
            hex!("0000000000000000000a558a61ddc8ee4e488d647a747fe4dcc362fe2026c620");

        // Performs 2^n hash iterations over it
        const N: u64 = 10;

        for i in 0..(1u64 << N) {
            // Print 1024 of the interstitial states
            // so that verification can be
            // parallelized

            if i % (1u64 << (N - 10)) == 0 {
                print!("{}: ", i);
                for b in cur_hash.iter() {
                    print!("{:02x}", b);
                }
                println!();
            }

            let mut h = Sha256::new();
            h.input(&cur_hash);
            h.result(&mut cur_hash);
        }

        print!("Final result of beacon: ");
        for b in cur_hash.iter() {
            print!("{:02x}", b);
        }
        println!();

        let mut digest = &cur_hash[..];

        let mut seed = [0u32; 8];
        for s in &mut seed {
            *s = digest
                .read_u32::<BigEndian>()
                .expect("digest is large enough for this to work");
        }

        ChaChaRng::from_seed(&seed)
    };

    println!("Done creating a beacon RNG");

    // Try to load challenge file from disk.
    let reader = OpenOptions::new()
        .read(true)
        .open(challenge_filename)
        .expect("unable open challenge file in this directory");

    {
        let metadata = reader
            .metadata()
            .expect("unable to get filesystem metadata for challenge file");
        let expected_challenge_length = match INPUT_IS_COMPRESSED {
            UseCompression::Yes => Bn256CeremonyParameters::CONTRIBUTION_BYTE_SIZE,
            UseCompression::No => Bn256CeremonyParameters::ACCUMULATOR_BYTE_SIZE,
        };

        if metadata.len() != (expected_challenge_length as u64) {
            panic!(
                "The size of challenge file should be {}, but it's {}, so something isn't right.",
                expected_challenge_length,
                metadata.len()
            );
        }
    }

    let readable_map = unsafe {
        MmapOptions::new()
            .map(&reader)
            .expect("unable to create a memory map for input")
    };

    // Create response file in this directory
    let writer = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(response_filename)
        .expect("unable to create response file in this directory");

    let required_output_length = match COMPRESS_THE_OUTPUT {
        UseCompression::Yes => Bn256CeremonyParameters::CONTRIBUTION_BYTE_SIZE,
        UseCompression::No => {
            Bn256CeremonyParameters::ACCUMULATOR_BYTE_SIZE
                + Bn256CeremonyParameters::PUBLIC_KEY_SIZE
        }
    };

    writer
        .set_len(required_output_length as u64)
        .expect("must make output file large enough");

    let mut writable_map = unsafe {
        MmapOptions::new()
            .map_mut(&writer)
            .expect("unable to create a memory map for output")
    };

    println!("Calculating previous contribution hash...");

    let current_accumulator_hash =
        BatchedAccumulator::<Bn256, Bn256CeremonyParameters>::calculate_hash(&readable_map);

    {
        println!("Contributing on top of the hash:");
        for line in current_accumulator_hash.as_slice().chunks(16) {
            print!("\t");
            for section in line.chunks(4) {
                for b in section {
                    print!("{:02x}", b);
                }
                print!(" ");
            }
            println!();
        }

        (&mut writable_map[0..])
            .write_all(current_accumulator_hash.as_slice())
            .expect("unable to write a challenge hash to mmap");

        writable_map
            .flush()
            .expect("unable to write hash to response file");
    }

    // Construct our keypair using the RNG we created above
    let (pubkey, privkey) = keypair(&mut rng, current_accumulator_hash.as_ref());

    // Perform the transformation
    println!("Computing and writing your contribution, this could take a while...");

    // this computes a transformation and writes it
    BatchedAccumulator::<Bn256, Bn256CeremonyParameters>::transform(
        &readable_map,
        &mut writable_map,
        INPUT_IS_COMPRESSED,
        COMPRESS_THE_OUTPUT,
        CHECK_INPUT_CORRECTNESS,
        &privkey,
    )
    .expect("must transform with the key");
    println!("Finishing writing your contribution to response file...");

    // Write the public key
    pubkey
        .write::<Bn256CeremonyParameters>(&mut writable_map, COMPRESS_THE_OUTPUT)
        .expect("unable to write public key");

    // Get the hash of the contribution, so the user can compare later
    let output_readonly = writable_map
        .make_read_only()
        .expect("must make a map readonly");
    let contribution_hash =
        BatchedAccumulator::<Bn256, Bn256CeremonyParameters>::calculate_hash(&output_readonly);

    print!(
        "Done!\n\n\
              Your contribution has been written to response file\n\n\
              The BLAKE2b hash of response file is:\n"
    );

    for line in contribution_hash.as_slice().chunks(16) {
        print!("\t");
        for section in line.chunks(4) {
            for b in section {
                print!("{:02x}", b);
            }
            print!(" ");
        }
        println!();
    }

    println!("Thank you for your participation, much appreciated! :)");
}
