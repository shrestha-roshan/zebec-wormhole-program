use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer as transfer_sol, Transfer as TransferSol};

use anchor_lang::solana_program::instruction::Instruction;

use anchor_spl::token::{approve, Approve};

use primitive_types::U256;
use sha3::Digest;

use byteorder::{BigEndian, WriteBytesExt};
use hex::decode;
use std::io::{Cursor, Write};
use std::str::FromStr;
mod constants;
mod context;
mod errors;
mod events;
mod payload;
mod portal;
mod state;
mod wormhole;

use constants::*;
use context::*;
use errors::*;
use events::*;
use payload::*;
use portal::*;
use wormhole::*;

use anchor_lang::solana_program::program::invoke_signed;

declare_id!("3qAAmNxTHxeL6pKDC6nb2PmoCE6hgZM2QXtS88gBm3yL");

#[program]
pub mod solana_project {

    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        ctx.accounts.config.owner = ctx.accounts.owner.key();
        ctx.accounts.config.nonce = 1;

        emit!(Initialized {
            owner: ctx.accounts.config.owner,
            nonce: ctx.accounts.config.nonce
        });
        Ok(())
    }

    pub fn register_chain(
        ctx: Context<RegisterChain>,
        chain_id: u16,
        emitter_addr: String,
    ) -> Result<()> {
        require!(
            emitter_addr.len() == EVM_CHAIN_ADDRESS_LENGTH,
            MessengerError::InvalidEmitterAddress
        );

        ctx.accounts.emitter_acc.chain_id = chain_id;
        ctx.accounts.emitter_acc.emitter_addr = emitter_addr.clone();

        emit!(RegisteredChain {
            chain_id: chain_id,
            emitter_addr: emitter_addr
        });
        Ok(())
    }

    pub fn initialize_pda(
        ctx: Context<InitializePDA>,
        _sender: [u8; 32],
        _chain_id: u16,
    ) -> Result<()> {
        //Hash a VAA Extract and derive a VAostedMA Key
        let vaa = PostedMessageData::try_from_slice(&ctx.accounts.core_bridge_vaa.data.borrow())?.0;
        let serialized_vaa = serialize_vaa(&vaa);

        let mut h = sha3::Keccak256::default();
        h.write_all(serialized_vaa.as_slice()).unwrap();
        let vaa_hash: [u8; 32] = h.finalize().into();

        let (vaa_key, _) = Pubkey::find_program_address(
            &[b"PostedVAA", &vaa_hash],
            &Pubkey::from_str(CORE_BRIDGE_ADDRESS).unwrap(),
        );

        require!(
            ctx.accounts.core_bridge_vaa.key() == vaa_key,
            MessengerError::VAAKeyMismatch
        );

        // Already checked that the SignedVaa is owned by core bridge in account constraint logic
        // Check that the emitter chain and address match up with the vaa
        require!(
            vaa.emitter_chain == ctx.accounts.emitter_acc.chain_id
                && vaa.emitter_address
                    == decode(ctx.accounts.emitter_acc.emitter_addr.as_str()).unwrap()[..],
            MessengerError::VAAEmitterMismatch
        );

        // Encoded String
        let encoded_str = vaa.payload.clone();

        // Decode Encoded String and Store Value based upon the code sent on message passing
        let code = get_u8(encoded_str[0..1].to_vec());

        require!(code == 18, MessengerError::InvalidPayload);
        let account_pda = Pubkey::find_program_address(
            &[&encoded_str[1..33], &vaa.emitter_chain.to_be_bytes()],
            ctx.program_id,
        )
        .0;
        require!(
            account_pda == ctx.accounts.pda_account.key(),
            MessengerError::InvalidPDAAccount
        );

        let to_chain_id = get_u256(encoded_str[33..65].to_vec());

        require!(
            to_chain_id == U256::from_str("1").unwrap(),
            MessengerError::InvalidToChainId
        );

        let rent_lamport = Rent::default().minimum_balance(1);

        let cpi_transfer_sol = TransferSol {
            from: ctx.accounts.zebec_eoa.to_account_info(),
            to: ctx.accounts.pda_account.to_account_info(),
        };
        let cpi_transfer_sol_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            cpi_transfer_sol,
        );
        transfer_sol(cpi_transfer_sol_ctx, rent_lamport + 5000000)?;

        emit!(InitializedPDA { pda: account_pda });

        Ok(())
    }

    pub fn initialize_pda_token_account(
        ctx: Context<InitializePDATokenAccount>,
        _sender: [u8; 32],
        _chain_id: u16,
    ) -> Result<()> {
        //Hash a VAA Extract and derive a VAostedMA Key
        let vaa = PostedMessageData::try_from_slice(&ctx.accounts.core_bridge_vaa.data.borrow())?.0;
        let serialized_vaa = serialize_vaa(&vaa);

        let mut h = sha3::Keccak256::default();
        h.write_all(serialized_vaa.as_slice()).unwrap();
        let vaa_hash: [u8; 32] = h.finalize().into();

        let vaa_key = Pubkey::find_program_address(
            &[b"PostedVAA", &vaa_hash],
            &Pubkey::from_str(CORE_BRIDGE_ADDRESS).unwrap(),
        )
        .0;

        require!(
            ctx.accounts.core_bridge_vaa.key() == vaa_key,
            MessengerError::VAAKeyMismatch
        );

        // Already checked that the SignedVaa is owned by core bridge in account constraint logic
        // Check that the emitter chain and address match up with the vaa
        require!(
            vaa.emitter_chain == ctx.accounts.emitter_acc.chain_id
                && vaa.emitter_address
                    == decode(ctx.accounts.emitter_acc.emitter_addr.as_str()).unwrap()[..],
            MessengerError::VAAEmitterMismatch
        );

        // Encoded String
        let encoded_str = vaa.payload.clone();

        // Decode Encoded String and Store   Value based upon the code sent on message passing
        let code = get_u8(encoded_str[0..1].to_vec());

        require!(code == 19, MessengerError::InvalidPayload);
        let account_pda = Pubkey::find_program_address(
            &[&encoded_str[1..33], &vaa.emitter_chain.to_be_bytes()],
            ctx.program_id,
        )
        .0;
        let token_mint_array: [u8; 32] = encoded_str[33..65].try_into().unwrap();
        let token_mint = Pubkey::new_from_array(token_mint_array);
        let to_chain_id = get_u256(encoded_str[65..97].to_vec());

        require!(
            to_chain_id == U256::from_str("1").unwrap(),
            MessengerError::InvalidToChainId
        );

        require!(
            account_pda == ctx.accounts.pda_account.key(),
            MessengerError::InvalidPDAAccount
        );
        require!(
            token_mint == ctx.accounts.token_mint.key(),
            MessengerError::MintKeyMismatch
        );

        emit!(InitializedPDATokenAccount {
            pda: account_pda,
            token_mint: token_mint,
        });
        Ok(())
    }

    //create and execute direct transfer native
    pub fn xstream_direct_transfer_native(
        ctx: Context<XstreamDirectTransferNative>,
        sender: [u8; 32],
        chain_id: u16,
        target_chain: u16,
        fee: u64,
    ) -> Result<()> {
        //Hash a VAA Extracts and derive a VAA Key
        let vaa = PostedMessageData::try_from_slice(&ctx.accounts.core_bridge_vaa.data.borrow())?.0;
        let serialized_vaa = serialize_vaa(&vaa);

        let mut h = sha3::Keccak256::default();
        h.write_all(serialized_vaa.as_slice()).unwrap();
        let vaa_hash: [u8; 32] = h.finalize().into();

        let vaa_key = Pubkey::find_program_address(
            &[b"PostedVAA", &vaa_hash],
            &Pubkey::from_str(CORE_BRIDGE_ADDRESS).unwrap(),
        )
        .0;

        require!(
            ctx.accounts.core_bridge_vaa.key() == vaa_key,
            MessengerError::VAAKeyMismatch
        );

        // Already checked that the SignedVaa is owned by core bridge in account constraint logic
        // Check that the emitter chain and address match up with the vaa
        require!(
            vaa.emitter_chain == ctx.accounts.emitter_acc.chain_id
                && vaa.emitter_address
                    == decode(ctx.accounts.emitter_acc.emitter_addr.as_str()).unwrap()[..],
            MessengerError::VAAEmitterMismatch
        );

        let payload = decode_xstream_direct(vaa.payload);

        //check sender
        let sender_stored = payload.sender;
        require!(sender == sender_stored, MessengerError::PdaSenderMismatch);

        //check receiver
        let receiver_stored = payload.receiver;

        //check pdaSender
        let chain_id_stored = chain_id;
        let chain_id_seed = &chain_id_stored.to_be_bytes();
        let (sender_derived_pubkey, _): (Pubkey, u8) =
            Pubkey::find_program_address(&[&sender, chain_id_seed], ctx.program_id);
        require!(
            ctx.accounts.pda_signer.key() == sender_derived_pubkey,
            MessengerError::SenderDerivedKeyMismatch
        );

        emit!(DirectTransferredNative {
            sender: sender,
            sender_chain: chain_id,
            target_chain: target_chain,
            receiver: receiver_stored,
        });

        transfer_native(
            ctx,
            sender,
            payload.amount,
            chain_id,
            target_chain,
            fee,
            receiver_stored,
        )
    }

    //create and execute direct transfer wrapped
    pub fn xstream_direct_transfer_wrapped(
        ctx: Context<XstreamDirectTransferWrapped>,
        sender: [u8; 32],
        sender_chain: u16,
        _token_address: Vec<u8>,
        _token_chain: u16,
        target_chain: u16,
        fee: u64,
    ) -> Result<()> {
        //Hash a VAA Extracts and derive a VAA Key
        let vaa = PostedMessageData::try_from_slice(&ctx.accounts.core_bridge_vaa.data.borrow())?.0;
        let serialized_vaa = serialize_vaa(&vaa);

        let mut h = sha3::Keccak256::default();
        h.write_all(serialized_vaa.as_slice()).unwrap();
        let vaa_hash: [u8; 32] = h.finalize().into();

        let vaa_key = Pubkey::find_program_address(
            &[b"PostedVAA", &vaa_hash],
            &Pubkey::from_str(CORE_BRIDGE_ADDRESS).unwrap(),
        )
        .0;

        require!(
            ctx.accounts.core_bridge_vaa.key() == vaa_key,
            MessengerError::VAAKeyMismatch
        );

        // Already checked that the SignedVaa is owned by core bridge in account constraint logic
        // Check that the emitter chain and address match up with the vaa
        require!(
            vaa.emitter_chain == ctx.accounts.emitter_acc.chain_id
                && vaa.emitter_address
                    == decode(ctx.accounts.emitter_acc.emitter_addr.as_str()).unwrap()[..],
            MessengerError::VAAEmitterMismatch
        );

        let payload = decode_xstream_direct(vaa.payload);
        //check sender
        let sender_stored = payload.sender;
        require!(sender == sender_stored, MessengerError::PdaSenderMismatch);

        //check receiver
        let receiver_stored = payload.receiver;

        //check pdaSender
        let chain_id_seed = &sender_chain.to_be_bytes();
        let (sender_derived_pubkey, _): (Pubkey, u8) =
            Pubkey::find_program_address(&[&sender, chain_id_seed], ctx.program_id);
        require!(
            ctx.accounts.pda_signer.key() == sender_derived_pubkey,
            MessengerError::SenderDerivedKeyMismatch
        );

        emit!(DirectTransferredWrapped {
            sender: sender,
            sender_chain: sender_chain,
            target_chain: target_chain,
            receiver: receiver_stored,
        });

        transfer_wrapped(
            ctx,
            sender,
            payload.amount,
            sender_chain,
            target_chain,
            fee,
            receiver_stored,
        )
    }

    pub fn xstream_withdraw(
        ctx: Context<XstreamWithdraw>,
        sender: [u8; 32],
        from_chain_id: u16,
    ) -> Result<()> {
        //Hash a VAA Extract and derive a VAA Key
        let vaa = PostedMessageData::try_from_slice(&ctx.accounts.core_bridge_vaa.data.borrow())?.0;
        let serialized_vaa = serialize_vaa(&vaa);

        let mut h = sha3::Keccak256::default();
        h.write_all(serialized_vaa.as_slice()).unwrap();
        let vaa_hash: [u8; 32] = h.finalize().into();

        let vaa_key = Pubkey::find_program_address(
            &[b"PostedVAA", &vaa_hash],
            &Pubkey::from_str(CORE_BRIDGE_ADDRESS).unwrap(),
        )
        .0;

        require!(
            ctx.accounts.core_bridge_vaa.key() == vaa_key,
            MessengerError::VAAKeyMismatch
        );

        // Already checked that the SignedVaa is owned by core bridge in account constraint logic
        // Check that the emitter chain and address match up with the vaa
        require!(
            vaa.emitter_chain == ctx.accounts.emitter_acc.chain_id
                && vaa.emitter_address
                    == decode(ctx.accounts.emitter_acc.emitter_addr.as_str()).unwrap()[..],
            MessengerError::VAAEmitterMismatch
        );

        let payload = decode_xstream_withdraw(vaa.payload);

        //check Mint passed
        let mint_pubkey_passed: Pubkey = ctx.accounts.mint.key();
        require!(
            mint_pubkey_passed == Pubkey::new(&payload.token_mint),
            MessengerError::MintKeyMismatch
        );

        //check data account
        let data_account_passed: Pubkey = ctx.accounts.data_account.key();
        require!(
            data_account_passed == Pubkey::new(&payload.data_account),
            MessengerError::DataAccountMismatch
        );

        //check sender
        let pda_sender_passed: Pubkey = ctx.accounts.source_account.key();
        let sender_stored = payload.depositor;

        //check receiver
        let pda_receiver_passed: Pubkey = ctx.accounts.dest_account.key();
        let receiver_stored = payload.withdrawer;
        require!(
            sender == receiver_stored,
            MessengerError::PdaReceiverMismatch
        );

        //check pdaSender
        let chain_id_stored = from_chain_id;
        let chain_id_seed = chain_id_stored.to_be_bytes();
        let sender_derived_pubkey: (Pubkey, u8) =
            Pubkey::find_program_address(&[&sender_stored, &chain_id_seed], ctx.program_id);
        require!(
            pda_sender_passed == sender_derived_pubkey.0,
            MessengerError::SenderDerivedKeyMismatch
        );

        //check pdaReceiver
        let receiver_derived_pubkey: (Pubkey, u8) =
            Pubkey::find_program_address(&[&receiver_stored, &chain_id_seed], ctx.program_id);
        require!(
            pda_receiver_passed == receiver_derived_pubkey.0,
            MessengerError::ReceiverDerivedKeyMismatch
        );

        let zebec_program = ctx.accounts.zebec_program.to_account_info();
        let zebec_accounts = zebec::cpi::accounts::TokenWithdrawStream {
            zebec_vault: ctx.accounts.zebec_vault.to_account_info(),
            dest_account: ctx.accounts.dest_account.to_account_info(),
            source_account: ctx.accounts.source_account.to_account_info(),
            fee_owner: ctx.accounts.fee_owner.to_account_info(),
            fee_vault_data: ctx.accounts.fee_vault_data.to_account_info(),
            fee_vault: ctx.accounts.fee_vault.to_account_info(),
            data_account: ctx.accounts.data_account.to_account_info(),
            withdraw_data: ctx.accounts.withdraw_data.to_account_info(),
            system_program: ctx.accounts.system_program.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
            associated_token_program: ctx.accounts.associated_token_program.to_account_info(),
            rent: ctx.accounts.rent.to_account_info(),
            mint: ctx.accounts.mint.to_account_info(),
            pda_account_token_account: ctx.accounts.pda_account_token_account.to_account_info(),
            dest_token_account: ctx.accounts.dest_token_account.to_account_info(),
            fee_receiver_token_account: ctx.accounts.fee_receiver_token_account.to_account_info(),
        };
        let bump = ctx.bumps.get("dest_account").unwrap().to_le_bytes();
        let seeds: &[&[_]] = &[&sender, &from_chain_id.to_be_bytes(), bump.as_ref()];
        let signer_seeds = &[&seeds[..]];
        let cpi_ctx = CpiContext::new_with_signer(zebec_program, zebec_accounts, signer_seeds);
        zebec::cpi::withdraw_token_stream(cpi_ctx)?;
        Ok(())
    }

    // Single Transaction methods starts from here
    pub fn xstream_start(
        ctx: Context<XstreamStart>,
        sender: [u8; 32],
        from_chain_id: u16,
    ) -> Result<()> {
        msg!("xstream start");
        //Hash a VAA Extract and derive a VAA Key
        let vaa = PostedMessageData::try_from_slice(&ctx.accounts.core_bridge_vaa.data.borrow())?.0;
        let serialized_vaa = serialize_vaa(&vaa);

        let mut h = sha3::Keccak256::default();
        h.write_all(serialized_vaa.as_slice()).unwrap();
        let vaa_hash: [u8; 32] = h.finalize().into();

        let vaa_key = Pubkey::find_program_address(
            &[b"PostedVAA", &vaa_hash],
            &Pubkey::from_str(CORE_BRIDGE_ADDRESS).unwrap(),
        )
        .0;

        require!(
            ctx.accounts.core_bridge_vaa.key() == vaa_key,
            MessengerError::VAAKeyMismatch
        );

        // Already checked that the SignedVaa is owned by core bridge in account constraint logic
        // Check that the emitter chain and address match up with the vaa
        require!(
            vaa.emitter_chain == ctx.accounts.emitter_acc.chain_id
                && vaa.emitter_address
                    == decode(ctx.accounts.emitter_acc.emitter_addr.as_str()).unwrap()[..],
            MessengerError::VAAEmitterMismatch
        );

        let payload = decode_xstream(vaa.payload);
        //let payload = XstreamStartPayload::try_from_slice(&vaa.payload[1..])?;

        //check Mint passed
        let mint_pubkey_passed: Pubkey = ctx.accounts.mint.key();
        require!(
            mint_pubkey_passed == Pubkey::new(&payload.token_mint),
            MessengerError::MintKeyMismatch
        );

        //check sender
        let pda_sender_passed: Pubkey = ctx.accounts.source_account.key();
        let sender_stored = payload.sender;
        require!(sender == sender_stored, MessengerError::PdaSenderMismatch);

        //check receiver
        let pda_receiver_passed: Pubkey = ctx.accounts.dest_account.key();
        let receiver_stored = payload.receiver;

        //check pdaSender
        let chain_id_stored: u16 = from_chain_id;
        let chain_id_seed = chain_id_stored.to_be_bytes();
        let sender_derived_pubkey: (Pubkey, u8) =
            Pubkey::find_program_address(&[&sender, &chain_id_seed], ctx.program_id);
        require!(
            pda_sender_passed == sender_derived_pubkey.0,
            MessengerError::SenderDerivedKeyMismatch
        );

        //check pdaReceivers
        let chain_id_seed = chain_id_stored.to_be_bytes();
        let receiver_derived_pubkey: (Pubkey, u8) =
            Pubkey::find_program_address(&[&receiver_stored, &chain_id_seed], ctx.program_id);
        require!(
            pda_receiver_passed == receiver_derived_pubkey.0,
            MessengerError::ReceiverDerivedKeyMismatch
        );

        let zebec_program = ctx.accounts.zebec_program.to_account_info();
        let zebec_accounts = zebec::cpi::accounts::TokenStream {
            dest_account: ctx.accounts.dest_account.to_account_info(),
            source_account: ctx.accounts.source_account.to_account_info(),
            fee_owner: ctx.accounts.fee_owner.to_account_info(),
            fee_vault_data: ctx.accounts.fee_vault_data.to_account_info(),
            fee_vault: ctx.accounts.fee_vault.to_account_info(),
            data_account: ctx.accounts.data_account.to_account_info(),
            withdraw_data: ctx.accounts.withdraw_data.to_account_info(),
            system_program: ctx.accounts.system_program.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
            rent: ctx.accounts.rent.to_account_info(),
            mint: ctx.accounts.mint.to_account_info(),
        };
        let bump = ctx.bumps.get("source_account").unwrap().to_le_bytes();
        let seeds: &[&[_]] = &[&sender, &from_chain_id.to_be_bytes(), bump.as_ref()];
        let signer_seeds = &[&seeds[..]];
        let cpi_ctx = CpiContext::new_with_signer(zebec_program, zebec_accounts, signer_seeds);
        zebec::cpi::token_stream(
            cpi_ctx,
            payload.start_time,
            payload.end_time,
            payload.amount,
            payload.can_cancel == 1,
            payload.can_update == 1,
        )?;
        Ok(())
    }

    pub fn xstream_update(
        ctx: Context<XstreamUpdate>,
        sender: [u8; 32],
        from_chain_id: u16,
    ) -> Result<()> {
        //Hash a VAA Extract and derive a VAA Key
        let vaa = PostedMessageData::try_from_slice(&ctx.accounts.core_bridge_vaa.data.borrow())?.0;
        let serialized_vaa = serialize_vaa(&vaa);

        let mut h = sha3::Keccak256::default();
        h.write_all(serialized_vaa.as_slice()).unwrap();
        let vaa_hash: [u8; 32] = h.finalize().into();

        let vaa_key = Pubkey::find_program_address(
            &[b"PostedVAA", &vaa_hash],
            &Pubkey::from_str(CORE_BRIDGE_ADDRESS).unwrap(),
        )
        .0;

        require!(
            ctx.accounts.core_bridge_vaa.key() == vaa_key,
            MessengerError::VAAKeyMismatch
        );

        // Already checked that the SignedVaa is owned by core bridge in account constraint logic
        // Check that the emitter chain and address match up with the vaa
        require!(
            vaa.emitter_chain == ctx.accounts.emitter_acc.chain_id
                && vaa.emitter_address
                    == decode(ctx.accounts.emitter_acc.emitter_addr.as_str()).unwrap()[..],
            MessengerError::VAAEmitterMismatch
        );

        let payload = decode_xstream_update(vaa.payload);

        //check Mint passed
        let mint_pubkey_passed: Pubkey = ctx.accounts.mint.key();
        require!(
            mint_pubkey_passed == Pubkey::new(&payload.token_mint),
            MessengerError::MintKeyMismatch
        );

        //check data account
        let data_account_passed: Pubkey = ctx.accounts.data_account.key();
        require!(
            data_account_passed == Pubkey::new(&payload.data_account),
            MessengerError::DataAccountMismatch
        );

        //check sender
        let pda_sender_passed: Pubkey = ctx.accounts.source_account.key();
        let sender_stored = payload.sender;

        //check receiver
        let pda_receiver_passed: Pubkey = ctx.accounts.dest_account.key();
        let receiver_stored = payload.receiver;
        require!(sender == sender_stored, MessengerError::PdaReceiverMismatch);

        //check pdaSender
        let chain_id_stored = from_chain_id;
        let chain_id_seed = chain_id_stored.to_be_bytes();
        let sender_derived_pubkey: (Pubkey, u8) =
            Pubkey::find_program_address(&[&sender_stored, &chain_id_seed], ctx.program_id);
        require!(
            pda_sender_passed == sender_derived_pubkey.0,
            MessengerError::SenderDerivedKeyMismatch
        );

        //check pdaReceiver
        let receiver_derived_pubkey: (Pubkey, u8) =
            Pubkey::find_program_address(&[&receiver_stored, &chain_id_seed], ctx.program_id);
        require!(
            pda_receiver_passed == receiver_derived_pubkey.0,
            MessengerError::ReceiverDerivedKeyMismatch
        );

        let zebec_program = ctx.accounts.zebec_program.to_account_info();
        let zebec_accounts = zebec::cpi::accounts::TokenStreamUpdate {
            dest_account: ctx.accounts.dest_account.to_account_info(),
            source_account: ctx.accounts.source_account.to_account_info(),
            data_account: ctx.accounts.data_account.to_account_info(),
            withdraw_data: ctx.accounts.withdraw_data.to_account_info(),
            mint: ctx.accounts.mint.to_account_info(),
        };
        let bump = ctx.bumps.get("source_account").unwrap().to_le_bytes();
        let seeds: &[&[_]] = &[&sender, &from_chain_id.to_be_bytes(), bump.as_ref()];
        let signer_seeds = &[&seeds[..]];
        let cpi_ctx = CpiContext::new_with_signer(zebec_program, zebec_accounts, signer_seeds);
        zebec::cpi::token_stream_update(
            cpi_ctx,
            payload.start_time,
            payload.end_time,
            payload.amount,
        )?;
        Ok(())
    }

    pub fn xstream_deposit(
        ctx: Context<XstreamDeposit>,
        sender: [u8; 32],
        from_chain_id: u16,
    ) -> Result<()> {
        //Hash a VAA Extract and derive a VAA Key
        let vaa = PostedMessageData::try_from_slice(&ctx.accounts.core_bridge_vaa.data.borrow())?.0;
        let serialized_vaa = serialize_vaa(&vaa);

        let mut h = sha3::Keccak256::default();
        h.write_all(serialized_vaa.as_slice()).unwrap();
        let vaa_hash: [u8; 32] = h.finalize().into();

        let vaa_key = Pubkey::find_program_address(
            &[b"PostedVAA", &vaa_hash],
            &Pubkey::from_str(CORE_BRIDGE_ADDRESS).unwrap(),
        )
        .0;

        require!(
            ctx.accounts.core_bridge_vaa.key() == vaa_key,
            MessengerError::VAAKeyMismatch
        );

        // Already checked that the SignedVaa is owned by core bridge in account constraint logic
        // Check that the emitter chain and address match up with the vaa
        require!(
            vaa.emitter_chain == ctx.accounts.emitter_acc.chain_id
                && vaa.emitter_address
                    == decode(ctx.accounts.emitter_acc.emitter_addr.as_str()).unwrap()[..],
            MessengerError::VAAEmitterMismatch
        );

        let payload = decode_xstream_deposit(vaa.payload);

        //check Mint passed
        let mint_pubkey_passed: Pubkey = Pubkey::new(&payload.token_mint);
        require!(
            mint_pubkey_passed == Pubkey::new(&payload.token_mint),
            MessengerError::MintKeyMismatch
        );

        //check sender
        let pda_sender_passed: Pubkey = ctx.accounts.source_account.key();
        let sender_stored = payload.sender;
        require!(sender == sender_stored, MessengerError::PdaSenderMismatch);

        //check pdaSender
        let chain_id_stored = from_chain_id;
        let chain_id_seed = &chain_id_stored.to_be_bytes();
        let derived_pubkey: (Pubkey, u8) =
            Pubkey::find_program_address(&[&sender, chain_id_seed], ctx.program_id);
        require!(
            pda_sender_passed == derived_pubkey.0,
            MessengerError::SenderDerivedKeyMismatch
        );

        let zebec_program = ctx.accounts.zebec_program.to_account_info();
        let zebec_accounts = zebec::cpi::accounts::TokenDeposit {
            zebec_vault: ctx.accounts.zebec_vault.to_account_info(),
            source_account: ctx.accounts.source_account.to_account_info(),
            system_program: ctx.accounts.system_program.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
            associated_token_program: ctx.accounts.associated_token_program.to_account_info(),
            rent: ctx.accounts.rent.to_account_info(),
            mint: ctx.accounts.mint.to_account_info(),
            pda_account_token_account: ctx.accounts.pda_account_token_account.to_account_info(),
            source_account_token_account: ctx
                .accounts
                .source_account_token_account
                .to_account_info(),
        };
        let bump = ctx.bumps.get("source_account").unwrap().to_le_bytes();
        let seeds: &[&[_]] = &[&sender, &from_chain_id.to_be_bytes(), bump.as_ref()];
        let signer_seeds = &[&seeds[..]];
        let cpi_ctx = CpiContext::new_with_signer(zebec_program, zebec_accounts, signer_seeds);
        zebec::cpi::deposit_token(cpi_ctx, payload.amount)?;
        Ok(())
    }

    pub fn xstream_sender_withdraw(
        ctx: Context<XstreamSenderWithdraw>,
        sender: [u8; 32],
        from_chain_id: u16,
    ) -> Result<()> {
        //Hash a VAA Extract and derive a VAA Key
        let vaa = PostedMessageData::try_from_slice(&ctx.accounts.core_bridge_vaa.data.borrow())?.0;
        let serialized_vaa = serialize_vaa(&vaa);

        let mut h = sha3::Keccak256::default();
        h.write_all(serialized_vaa.as_slice()).unwrap();
        let vaa_hash: [u8; 32] = h.finalize().into();

        let vaa_key = Pubkey::find_program_address(
            &[b"PostedVAA", &vaa_hash],
            &Pubkey::from_str(CORE_BRIDGE_ADDRESS).unwrap(),
        )
        .0;

        require!(
            ctx.accounts.core_bridge_vaa.key() == vaa_key,
            MessengerError::VAAKeyMismatch
        );

        // Already checked that the SignedVaa is owned by core bridge in account constraint logic
        // Check that the emitter chain and address match up with the vaa
        require!(
            vaa.emitter_chain == ctx.accounts.emitter_acc.chain_id
                && vaa.emitter_address
                    == decode(ctx.accounts.emitter_acc.emitter_addr.as_str()).unwrap()[..],
            MessengerError::VAAEmitterMismatch
        );

        let payload = decode_deposit_withdraw(vaa.payload);

        //check Mint passed
        let mint_pubkey_passed: Pubkey = ctx.accounts.mint.key();
        require!(
            mint_pubkey_passed == Pubkey::new(&payload.token_mint),
            MessengerError::MintKeyMismatch
        );

        //check sender
        let pda_sender_passed: Pubkey = ctx.accounts.source_account.key();
        let sender_stored = payload.withdrawer;
        require!(sender == sender_stored, MessengerError::PdaSenderMismatch);

        //check pdaSender
        let chain_id_stored = from_chain_id;
        let chain_id_seed = &chain_id_stored.to_be_bytes();
        let sender_derived_pubkey: (Pubkey, u8) =
            Pubkey::find_program_address(&[&sender, chain_id_seed], ctx.program_id);
        require!(
            pda_sender_passed == sender_derived_pubkey.0,
            MessengerError::SenderDerivedKeyMismatch
        );

        let zebec_program = ctx.accounts.zebec_program.to_account_info();
        let zebec_accounts = zebec::cpi::accounts::InitializerTokenWithdrawal {
            zebec_vault: ctx.accounts.zebec_vault.to_account_info(),
            source_account: ctx.accounts.source_account.to_account_info(),
            withdraw_data: ctx.accounts.withdraw_data.to_account_info(),
            system_program: ctx.accounts.system_program.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
            associated_token_program: ctx.accounts.associated_token_program.to_account_info(),
            rent: ctx.accounts.rent.to_account_info(),
            mint: ctx.accounts.mint.to_account_info(),
            source_account_token_account: ctx
                .accounts
                .source_account_token_account
                .to_account_info(),
            pda_account_token_account: ctx.accounts.pda_account_token_account.to_account_info(),
        };
        let bump = ctx.bumps.get("source_account").unwrap().to_le_bytes();
        let seeds: &[&[_]] = &[&sender, &from_chain_id.to_be_bytes(), bump.as_ref()];
        let signer_seeds = &[&seeds[..]];
        let cpi_ctx = CpiContext::new_with_signer(zebec_program, zebec_accounts, signer_seeds);
        zebec::cpi::token_withdrawal(cpi_ctx, payload.amount)?;
        Ok(())
    }

    pub fn xstream_pause(
        ctx: Context<XstreamPause>,
        sender: [u8; 32],
        from_chain_id: u16,
    ) -> Result<()> {
        //Hash a VAA Extract and derive a VAA Key
        let vaa = PostedMessageData::try_from_slice(&ctx.accounts.core_bridge_vaa.data.borrow())?.0;
        let serialized_vaa = serialize_vaa(&vaa);

        let mut h = sha3::Keccak256::default();
        h.write_all(serialized_vaa.as_slice()).unwrap();
        let vaa_hash: [u8; 32] = h.finalize().into();

        let vaa_key = Pubkey::find_program_address(
            &[b"PostedVAA", &vaa_hash],
            &Pubkey::from_str(CORE_BRIDGE_ADDRESS).unwrap(),
        )
        .0;

        require!(
            ctx.accounts.core_bridge_vaa.key() == vaa_key,
            MessengerError::VAAKeyMismatch
        );

        // Already checked that the SignedVaa is owned by core bridge in account constraint logic
        // Check that the emitter chain and address match up with the vaa
        require!(
            vaa.emitter_chain == ctx.accounts.emitter_acc.chain_id
                && vaa.emitter_address
                    == decode(ctx.accounts.emitter_acc.emitter_addr.as_str()).unwrap()[..],
            MessengerError::VAAEmitterMismatch
        );

        let payload = decode_xstream_pause(vaa.payload);

        //check data account
        let data_account_passed: Pubkey = ctx.accounts.data_account.key();
        require!(
            data_account_passed == Pubkey::new(&payload.data_account),
            MessengerError::DataAccountMismatch
        );

        //check sender
        let pda_sender_passed: Pubkey = ctx.accounts.source_account.key();
        let sender_stored = payload.depositor;
        require!(sender == sender_stored, MessengerError::PdaSenderMismatch);

        //check receiver
        let pda_receiver_passed: Pubkey = ctx.accounts.dest_account.key();
        let receiver_stored = payload.receiver;

        //check pdaSender
        let chain_id_stored = from_chain_id;
        let chain_id_seed = &chain_id_stored.to_be_bytes();
        let sender_derived_pubkey: (Pubkey, u8) =
            Pubkey::find_program_address(&[&sender, chain_id_seed], ctx.program_id);
        require!(
            pda_sender_passed == sender_derived_pubkey.0,
            MessengerError::SenderDerivedKeyMismatch
        );

        //check pdaReceiver
        let chain_id_seed = chain_id_stored.to_be_bytes();
        let receiver_derived_pubkey: (Pubkey, u8) =
            Pubkey::find_program_address(&[&receiver_stored, &chain_id_seed], ctx.program_id);
        require!(
            pda_receiver_passed == receiver_derived_pubkey.0,
            MessengerError::ReceiverDerivedKeyMismatch
        );

        let zebec_program = ctx.accounts.zebec_program.to_account_info();
        let zebec_accounts = zebec::cpi::accounts::PauseTokenStream {
            data_account: ctx.accounts.data_account.to_account_info(),
            withdraw_data: ctx.accounts.withdraw_data.to_account_info(),
            mint: ctx.accounts.mint.to_account_info(),
            sender: ctx.accounts.source_account.to_account_info(),
            receiver: ctx.accounts.dest_account.to_account_info(),
        };
        let bump = ctx.bumps.get("source_account").unwrap().to_le_bytes();
        let seeds: &[&[_]] = &[&sender, &from_chain_id.to_be_bytes(), bump.as_ref()];
        let signer_seeds = &[&seeds[..]];
        let cpi_ctx = CpiContext::new_with_signer(zebec_program, zebec_accounts, signer_seeds);
        zebec::cpi::pause_resume_token_stream(cpi_ctx)?;
        Ok(())
    }

    pub fn xstream_cancel(
        ctx: Context<XstreamCancel>,
        sender: [u8; 32],
        from_chain_id: u16,
    ) -> Result<()> {
        //Hash a VAA Extract and derive a VAA Key
        let vaa = PostedMessageData::try_from_slice(&ctx.accounts.core_bridge_vaa.data.borrow())?.0;
        let serialized_vaa = serialize_vaa(&vaa);

        let mut h = sha3::Keccak256::default();
        h.write_all(serialized_vaa.as_slice()).unwrap();
        let vaa_hash: [u8; 32] = h.finalize().into();

        let vaa_key = Pubkey::find_program_address(
            &[b"PostedVAA", &vaa_hash],
            &Pubkey::from_str(CORE_BRIDGE_ADDRESS).unwrap(),
        )
        .0;

        require!(
            ctx.accounts.core_bridge_vaa.key() == vaa_key,
            MessengerError::VAAKeyMismatch
        );

        // Already checked that the SignedVaa is owned by core bridge in account constraint logic
        // Check that the emitter chain and address match up with the vaa
        require!(
            vaa.emitter_chain == ctx.accounts.emitter_acc.chain_id
                && vaa.emitter_address
                    == decode(ctx.accounts.emitter_acc.emitter_addr.as_str()).unwrap()[..],
            MessengerError::VAAEmitterMismatch
        );

        let payload = decode_xstream_cancel(vaa.payload);

        //check Mint passed
        let mint_pubkey_passed: Pubkey = ctx.accounts.mint.key();
        require!(
            mint_pubkey_passed == Pubkey::new(&payload.token_mint),
            MessengerError::MintKeyMismatch
        );

        //check data account
        let data_account_passed: Pubkey = ctx.accounts.data_account.key();
        require!(
            data_account_passed == Pubkey::new(&payload.data_account),
            MessengerError::DataAccountMismatch
        );

        //check sender
        let pda_sender_passed: Pubkey = ctx.accounts.source_account.key();
        let sender_stored = payload.depositor;
        require!(sender == sender_stored, MessengerError::PdaSenderMismatch);

        //check receiver
        let pda_receiver_passed: Pubkey = ctx.accounts.dest_account.key();
        let receiver_stored = payload.receiver;

        //check pdaSender
        let chain_id_stored = from_chain_id;
        let chain_id_seed = &chain_id_stored.to_be_bytes();
        let sender_derived_pubkey: (Pubkey, u8) =
            Pubkey::find_program_address(&[&sender, chain_id_seed], ctx.program_id);
        require!(
            pda_sender_passed == sender_derived_pubkey.0,
            MessengerError::SenderDerivedKeyMismatch
        );

        //check pdaReceiver
        let chain_id_stored = from_chain_id;
        let chain_id_seed = chain_id_stored.to_be_bytes();
        let receiver_derived_pubkey: (Pubkey, u8) =
            Pubkey::find_program_address(&[&receiver_stored, &chain_id_seed], ctx.program_id);
        require!(
            pda_receiver_passed == receiver_derived_pubkey.0,
            MessengerError::ReceiverDerivedKeyMismatch
        );

        let zebec_program = ctx.accounts.zebec_program.to_account_info();
        let zebec_accounts = zebec::cpi::accounts::CancelTokenStream {
            zebec_vault: ctx.accounts.zebec_vault.to_account_info(),
            dest_account: ctx.accounts.dest_account.to_account_info(),
            source_account: ctx.accounts.source_account.to_account_info(),
            fee_owner: ctx.accounts.fee_owner.to_account_info(),
            fee_vault_data: ctx.accounts.fee_vault_data.to_account_info(),
            fee_vault: ctx.accounts.fee_vault.to_account_info(),
            data_account: ctx.accounts.data_account.to_account_info(),
            withdraw_data: ctx.accounts.withdraw_data.to_account_info(),
            system_program: ctx.accounts.system_program.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
            associated_token_program: ctx.accounts.associated_token_program.to_account_info(),
            rent: ctx.accounts.rent.to_account_info(),
            mint: ctx.accounts.mint.to_account_info(),
            pda_account_token_account: ctx.accounts.pda_account_token_account.to_account_info(),
            dest_token_account: ctx.accounts.dest_token_account.to_account_info(),
            fee_receiver_token_account: ctx.accounts.fee_receiver_token_account.to_account_info(),
        };
        let bump = ctx.bumps.get("source_account").unwrap().to_le_bytes();
        let seeds: &[&[_]] = &[&sender, &from_chain_id.to_be_bytes(), bump.as_ref()];
        let signer_seeds = &[&seeds[..]];
        let cpi_ctx = CpiContext::new_with_signer(zebec_program, zebec_accounts, signer_seeds);
        zebec::cpi::cancel_token_stream(cpi_ctx)?;
        Ok(())
    }

    pub fn instant_transfer(
        ctx: Context<XstreamInstant>,
        sender: [u8; 32],
        from_chain_id: u16,
    ) -> Result<()> {
        //Hash a VAA Extracts and derive a VAA Key
        let vaa = PostedMessageData::try_from_slice(&ctx.accounts.core_bridge_vaa.data.borrow())?.0;
        let serialized_vaa = serialize_vaa(&vaa);

        let mut h = sha3::Keccak256::default();
        h.write_all(serialized_vaa.as_slice()).unwrap();
        let vaa_hash: [u8; 32] = h.finalize().into();

        let vaa_key = Pubkey::find_program_address(
            &[b"PostedVAA", &vaa_hash],
            &Pubkey::from_str(CORE_BRIDGE_ADDRESS).unwrap(),
        )
        .0;

        require!(
            ctx.accounts.core_bridge_vaa.key() == vaa_key,
            MessengerError::VAAKeyMismatch
        );

        // Already checked that the SignedVaa is owned by core bridge in account constraint logic
        // Check that the emitter chain and address match up with the vaa
        require!(
            vaa.emitter_chain == ctx.accounts.emitter_acc.chain_id
                && vaa.emitter_address
                    == decode(ctx.accounts.emitter_acc.emitter_addr.as_str()).unwrap()[..],
            MessengerError::VAAEmitterMismatch
        );

        let payload = decode_xstream_instant(vaa.payload);

        //check Mint passed
        let mint_pubkey_passed: Pubkey = ctx.accounts.mint.key();
        require!(
            mint_pubkey_passed == Pubkey::new(&payload.token_mint),
            MessengerError::MintKeyMismatch
        );

        //check sender
        let pda_sender_passed: Pubkey = ctx.accounts.source_account.key();
        let sender_stored = payload.sender;
        require!(sender == sender_stored, MessengerError::PdaSenderMismatch);

        //check receiver
        let pda_receiver_passed: Pubkey = ctx.accounts.dest_account.key();
        let receiver_stored = payload.receiver;

        //check pdaSender
        let chain_id_stored = from_chain_id;
        let chain_id_seed = &chain_id_stored.to_be_bytes();
        let sender_derived_pubkey: (Pubkey, u8) =
            Pubkey::find_program_address(&[&sender, chain_id_seed], ctx.program_id);
        require!(
            pda_sender_passed == sender_derived_pubkey.0,
            MessengerError::SenderDerivedKeyMismatch
        );

        //check pdaReceiver
        let chain_id_stored = from_chain_id;
        let chain_id_seed = chain_id_stored.to_be_bytes();
        let receiver_derived_pubkey: (Pubkey, u8) =
            Pubkey::find_program_address(&[&receiver_stored, &chain_id_seed], ctx.program_id);
        require!(
            pda_receiver_passed == receiver_derived_pubkey.0,
            MessengerError::ReceiverDerivedKeyMismatch
        );

        let zebec_program = ctx.accounts.zebec_program.to_account_info();
        let zebec_accounts = zebec::cpi::accounts::TokenInstantTransfer {
            zebec_vault: ctx.accounts.zebec_vault.to_account_info(),
            dest_account: ctx.accounts.dest_account.to_account_info(),
            source_account: ctx.accounts.source_account.to_account_info(),
            withdraw_data: ctx.accounts.withdraw_data.to_account_info(),
            system_program: ctx.accounts.system_program.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
            associated_token_program: ctx.accounts.associated_token_program.to_account_info(),
            rent: ctx.accounts.rent.to_account_info(),
            mint: ctx.accounts.mint.to_account_info(),
            pda_account_token_account: ctx.accounts.pda_account_token_account.to_account_info(),
            dest_token_account: ctx.accounts.dest_token_account.to_account_info(),
        };
        let bump = ctx.bumps.get("source_account").unwrap().to_le_bytes();
        let seeds: &[&[_]] = &[&sender, &from_chain_id.to_be_bytes(), bump.as_ref()];
        let signer_seeds = &[&seeds[..]];
        let cpi_ctx = CpiContext::new_with_signer(zebec_program, zebec_accounts, signer_seeds);
        zebec::cpi::instant_token_transfer(cpi_ctx, payload.amount)?;
        Ok(())
    }
}

fn transfer_wrapped(
    ctx: Context<XstreamDirectTransferWrapped>,
    sender: [u8; 32],
    amount: u64,
    sender_chain: u16,
    target_chain: u16,
    fee: u64,
    receiver: [u8; 32],
) -> Result<()> {
    //Check EOA
    require!(
        ctx.accounts.config.owner == ctx.accounts.zebec_eoa.key(),
        MessengerError::InvalidCaller
    );
    let bump = ctx.bumps.get("pda_signer").unwrap().to_le_bytes();

    let signer_seeds: &[&[&[u8]]] = &[&[&sender, &sender_chain.to_be_bytes(), &bump]];

    let approve_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        Approve {
            to: ctx.accounts.from.to_account_info(),
            delegate: ctx.accounts.portal_authority_signer.to_account_info(),
            authority: ctx.accounts.pda_signer.to_account_info(),
        },
        signer_seeds,
    );

    // Delgate transfer authority to Token Bridge for the tokens
    approve(approve_ctx, amount)?;

    let target_address: [u8; 32] = receiver.as_slice().try_into().unwrap();
    // Instruction
    let transfer_ix = Instruction {
        program_id: Pubkey::from_str(TOKEN_BRIDGE_ADDRESS).unwrap(),
        accounts: vec![
            AccountMeta::new(ctx.accounts.zebec_eoa.key(), true),
            AccountMeta::new_readonly(ctx.accounts.portal_config.key(), false),
            AccountMeta::new(ctx.accounts.from.key(), false),
            AccountMeta::new_readonly(ctx.accounts.pda_signer.key(), true),
            AccountMeta::new(ctx.accounts.wrapped_mint.key(), false),
            AccountMeta::new_readonly(ctx.accounts.wrapped_meta.key(), false),
            AccountMeta::new_readonly(ctx.accounts.portal_authority_signer.key(), false),
            AccountMeta::new(ctx.accounts.bridge_config.key(), false),
            AccountMeta::new(ctx.accounts.portal_message.key(), true),
            AccountMeta::new_readonly(ctx.accounts.portal_emitter.key(), false),
            AccountMeta::new(ctx.accounts.portal_sequence.key(), false),
            AccountMeta::new(ctx.accounts.bridge_fee_collector.key(), false),
            AccountMeta::new_readonly(ctx.accounts.clock.key(), false),
            // Dependencies
            AccountMeta::new_readonly(ctx.accounts.rent.key(), false),
            AccountMeta::new_readonly(ctx.accounts.system_program.key(), false),
            // Program
            AccountMeta::new_readonly(ctx.accounts.core_bridge_program.key(), false),
            AccountMeta::new_readonly(ctx.accounts.token_program.key(), false),
        ],
        data: (
            crate::portal::Instruction::TransferWrapped,
            TransferWrappedData {
                nonce: ctx.accounts.config.nonce,
                amount,
                fee,
                target_address,
                target_chain,
            },
        )
            .try_to_vec()?,
    };

    // Accounts
    let transfer_accs = vec![
        ctx.accounts.zebec_eoa.to_account_info(),
        ctx.accounts.portal_config.to_account_info(),
        ctx.accounts.from.to_account_info(),
        ctx.accounts.pda_signer.to_account_info(),
        ctx.accounts.wrapped_mint.to_account_info(),
        ctx.accounts.wrapped_meta.to_account_info(),
        ctx.accounts.portal_authority_signer.to_account_info(),
        ctx.accounts.bridge_config.to_account_info(),
        ctx.accounts.portal_message.to_account_info(),
        ctx.accounts.portal_emitter.to_account_info(),
        ctx.accounts.portal_sequence.to_account_info(),
        ctx.accounts.bridge_fee_collector.to_account_info(),
        ctx.accounts.clock.to_account_info(),
        // Dependencies
        ctx.accounts.rent.to_account_info(),
        ctx.accounts.system_program.to_account_info(),
        // Program
        ctx.accounts.core_bridge_program.to_account_info(),
        ctx.accounts.token_program.to_account_info(),
    ];

    invoke_signed(&transfer_ix, &transfer_accs, signer_seeds)?;

    let sum = ctx.accounts.config.nonce.checked_add(1);
    match sum {
        None => return Err(MessengerError::Overflow.into()),
        Some(val) => ctx.accounts.config.nonce = val,
    }

    Ok(())
}

//transfer
fn transfer_native(
    ctx: Context<XstreamDirectTransferNative>,
    sender: [u8; 32],
    amount: u64,
    sender_chain: u16,
    target_chain: u16,
    fee: u64,
    receiver: [u8; 32],
) -> Result<()> {
    //Check EOA
    require!(
        ctx.accounts.config.owner == ctx.accounts.zebec_eoa.key(),
        MessengerError::InvalidCaller
    );

    let bump = ctx.bumps.get("pda_signer").unwrap().to_le_bytes();

    let signer_seeds: &[&[&[u8]]] = &[&[&sender, &sender_chain.to_be_bytes(), &bump]];

    let approve_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        Approve {
            to: ctx.accounts.from.to_account_info(),
            delegate: ctx.accounts.portal_authority_signer.to_account_info(),
            authority: ctx.accounts.pda_signer.to_account_info(),
        },
        signer_seeds,
    );

    // Delgate transfer authority to Token Bridge for the tokens
    approve(approve_ctx, amount)?;

    let target_address: [u8; 32] = receiver.as_slice().try_into().unwrap();
    // Instruction
    let transfer_ix = Instruction {
        program_id: Pubkey::from_str(TOKEN_BRIDGE_ADDRESS).unwrap(),
        accounts: vec![
            AccountMeta::new(ctx.accounts.zebec_eoa.key(), true),
            AccountMeta::new_readonly(ctx.accounts.portal_config.key(), false),
            AccountMeta::new(ctx.accounts.from.key(), false),
            AccountMeta::new(ctx.accounts.mint.key(), false),
            AccountMeta::new(ctx.accounts.portal_custody.key(), false),
            AccountMeta::new_readonly(ctx.accounts.portal_authority_signer.key(), false),
            AccountMeta::new_readonly(ctx.accounts.portal_custody_signer.key(), false),
            AccountMeta::new(ctx.accounts.bridge_config.key(), false),
            AccountMeta::new(ctx.accounts.portal_message.key(), true),
            AccountMeta::new_readonly(ctx.accounts.portal_emitter.key(), false),
            AccountMeta::new(ctx.accounts.portal_sequence.key(), false),
            AccountMeta::new(ctx.accounts.bridge_fee_collector.key(), false),
            AccountMeta::new_readonly(ctx.accounts.clock.key(), false),
            // Dependencies
            AccountMeta::new_readonly(ctx.accounts.rent.key(), false),
            AccountMeta::new_readonly(ctx.accounts.system_program.key(), false),
            // Program
            AccountMeta::new_readonly(ctx.accounts.core_bridge_program.key(), false),
            AccountMeta::new_readonly(ctx.accounts.token_program.key(), false),
        ],
        data: (
            crate::portal::Instruction::TransferNative,
            TransferNativeData {
                nonce: ctx.accounts.config.nonce,
                amount,
                fee,
                target_address,
                target_chain,
            },
        )
            .try_to_vec()?,
    };

    // Accounts
    let transfer_accs = vec![
        ctx.accounts.zebec_eoa.to_account_info(),
        ctx.accounts.portal_config.to_account_info(),
        ctx.accounts.from.to_account_info(),
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.portal_custody.to_account_info(),
        ctx.accounts.portal_authority_signer.to_account_info(),
        ctx.accounts.portal_custody_signer.to_account_info(),
        ctx.accounts.bridge_config.to_account_info(),
        ctx.accounts.portal_message.to_account_info(),
        ctx.accounts.portal_emitter.to_account_info(),
        ctx.accounts.portal_sequence.to_account_info(),
        ctx.accounts.bridge_fee_collector.to_account_info(),
        ctx.accounts.clock.to_account_info(),
        // Dependencies
        ctx.accounts.rent.to_account_info(),
        ctx.accounts.system_program.to_account_info(),
        // Program
        ctx.accounts.core_bridge_program.to_account_info(),
        ctx.accounts.token_program.to_account_info(),
    ];

    invoke_signed(&transfer_ix, &transfer_accs, signer_seeds)?;

    let sum = ctx.accounts.config.nonce.checked_add(1);
    match sum {
        None => return Err(MessengerError::Overflow.into()),
        Some(val) => ctx.accounts.config.nonce = val,
    }

    Ok(())
}

fn get_u64(data_bytes: Vec<u8>) -> u64 {
    let data_u8 = <[u8; 8]>::try_from(data_bytes).unwrap();
    u64::from_be_bytes(data_u8)
}

fn get_u256(data_bytes: Vec<u8>) -> U256 {
    let data_u8 = <[u8; 32]>::try_from(data_bytes).unwrap();
    U256::from_big_endian(&data_u8)
}

fn get_u8(data_bytes: Vec<u8>) -> u64 {
    let prefix_bytes = vec![0; 7];
    let joined_bytes = [prefix_bytes, data_bytes].concat();
    let data_u8 = <[u8; 8]>::try_from(joined_bytes).unwrap();
    u64::from_be_bytes(data_u8)
}

fn get_u32_array(data_bytes: Vec<u8>) -> [u8; 32] {
    let data_result = data_bytes.try_into().unwrap();
    return data_result;
}

// Convert a full VAA structure into the serialization of its unique components, this structure is
// what is hashed and verified by Guardians.
pub fn serialize_vaa(vaa: &MessageData) -> Vec<u8> {
    let mut v = Cursor::new(Vec::new());
    v.write_u32::<BigEndian>(vaa.vaa_time).unwrap();
    v.write_u32::<BigEndian>(vaa.nonce).unwrap();
    v.write_u16::<BigEndian>(vaa.emitter_chain as u16).unwrap();
    v.write_all(&vaa.emitter_address).unwrap();
    v.write_u64::<BigEndian>(vaa.sequence).unwrap();
    v.write_u8(vaa.consistency_level).unwrap();
    v.write_all(&vaa.payload).unwrap();
    v.into_inner()
}

fn decode_xstream(encoded_str: Vec<u8>) -> XstreamStartPayload {
    let start_time = get_u64(encoded_str[1..9].to_vec());
    let end_time = get_u64(encoded_str[9..17].to_vec());
    let amount = get_u64(encoded_str[17..25].to_vec());
    let to_chain_id = get_u32_array(encoded_str[25..57].to_vec());
    let sender = get_u32_array(encoded_str[57..89].to_vec());
    let receiver = get_u32_array(encoded_str[89..121].to_vec());
    let can_cancel = get_u64(encoded_str[121..129].to_vec());
    let can_update = get_u64(encoded_str[129..137].to_vec());
    let token_mint = get_u32_array(encoded_str[137..169].to_vec());

    let stream_payload = XstreamStartPayload {
        start_time,
        end_time,
        amount,
        to_chain_id,
        sender,
        receiver,
        can_update,
        can_cancel,
        token_mint,
    };
    stream_payload
}

fn decode_xstream_withdraw(encoded_str: Vec<u8>) -> XstreamWithdrawPayload {
    let to_chain_id = get_u32_array(encoded_str[1..33].to_vec());
    let withdrawer = get_u32_array(encoded_str[33..65].to_vec());
    let token_mint = get_u32_array(encoded_str[65..97].to_vec());
    let depositor = get_u32_array(encoded_str[97..129].to_vec());
    let data_account = get_u32_array(encoded_str[129..161].to_vec());

    let payload = XstreamWithdrawPayload {
        to_chain_id,
        withdrawer,
        token_mint,
        depositor,
        data_account,
    };
    payload
}

fn decode_xstream_deposit(encoded_str: Vec<u8>) -> XstreamDepositPayload {
    let amount = get_u64(encoded_str[1..9].to_vec());
    let to_chain_id = get_u32_array(encoded_str[9..41].to_vec());
    let sender = get_u32_array(encoded_str[41..73].to_vec());
    let token_mint = get_u32_array(encoded_str[73..105].to_vec());

    let payload = XstreamDepositPayload {
        amount,
        to_chain_id,
        sender,
        token_mint,
    };
    payload
}

fn decode_xstream_update(encoded_str: Vec<u8>) -> XstreamUpdatePayload {
    let start_time = get_u64(encoded_str[1..9].to_vec());
    let end_time = get_u64(encoded_str[9..17].to_vec());
    let amount = get_u64(encoded_str[17..25].to_vec());
    let to_chain_id = get_u32_array(encoded_str[25..57].to_vec());
    let sender = get_u32_array(encoded_str[57..89].to_vec());
    let receiver = get_u32_array(encoded_str[89..121].to_vec());
    let token_mint = get_u32_array(encoded_str[121..153].to_vec());
    let data_account = get_u32_array(encoded_str[153..185].to_vec());

    let payload = XstreamUpdatePayload {
        start_time,
        end_time,
        amount,
        to_chain_id,
        sender,
        receiver,
        token_mint,
        data_account,
    };
    payload
}

fn decode_xstream_pause(encoded_str: Vec<u8>) -> XstreamPausePayload {
    let to_chain_id = get_u32_array(encoded_str[1..33].to_vec());
    let depositor = get_u32_array(encoded_str[33..65].to_vec());
    let token_mint = get_u32_array(encoded_str[65..97].to_vec());
    let receiver = get_u32_array(encoded_str[97..129].to_vec());
    let data_account = get_u32_array(encoded_str[129..161].to_vec());

    let payload = XstreamPausePayload {
        to_chain_id,
        depositor,
        token_mint,
        receiver,
        data_account,
    };
    payload
}

fn decode_xstream_cancel(encoded_str: Vec<u8>) -> XstreamCancelPayload {
    let to_chain_id = get_u32_array(encoded_str[1..33].to_vec());
    let depositor = get_u32_array(encoded_str[33..65].to_vec());
    let token_mint = get_u32_array(encoded_str[65..97].to_vec());
    let receiver = get_u32_array(encoded_str[97..129].to_vec());
    let data_account = get_u32_array(encoded_str[129..161].to_vec());

    let payload = XstreamCancelPayload {
        to_chain_id,
        depositor,
        token_mint,
        receiver,
        data_account,
    };
    payload
}

fn decode_deposit_withdraw(encoded_str: Vec<u8>) -> XstreamWithdrawDepositPayload {
    let amount = get_u64(encoded_str[1..9].to_vec());
    let to_chain_id = get_u32_array(encoded_str[9..41].to_vec());
    let withdrawer = get_u32_array(encoded_str[41..73].to_vec());
    let token_mint = get_u32_array(encoded_str[73..105].to_vec());

    let payload = XstreamWithdrawDepositPayload {
        amount,
        to_chain_id,
        withdrawer,
        token_mint,
    };
    payload
}

fn decode_xstream_instant(encoded_str: Vec<u8>) -> XstreamInstantTransferPayload {
    let amount = get_u64(encoded_str[1..9].to_vec());
    let to_chain_id = get_u32_array(encoded_str[9..41].to_vec());
    let sender = get_u32_array(encoded_str[41..73].to_vec());
    let token_mint = get_u32_array(encoded_str[73..105].to_vec());
    let receiver = get_u32_array(encoded_str[105..137].to_vec());

    let payload = XstreamInstantTransferPayload {
        amount,
        to_chain_id,
        sender,
        token_mint,
        receiver,
    };
    payload
}

fn decode_xstream_direct(encoded_str: Vec<u8>) -> XstreamDirectTransferPayload {
    let amount = get_u64(encoded_str[1..9].to_vec());
    let to_chain_id = get_u32_array(encoded_str[9..41].to_vec());
    let sender = get_u32_array(encoded_str[41..73].to_vec());
    let token_mint = get_u32_array(encoded_str[73..105].to_vec());
    let receiver = get_u32_array(encoded_str[105..137].to_vec());

    let payload = XstreamDirectTransferPayload {
        amount,
        to_chain_id,
        sender,
        token_mint,
        receiver,
    };
    payload
}
