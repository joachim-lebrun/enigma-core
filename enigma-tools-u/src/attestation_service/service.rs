use serde_json;
use serde_json::{Value};
use reqwest;
use failure::Error;
use common_u::errors;
use attestation_service;
use rlp;
use base64;
use std::io::Read;
use std::mem;
use std::string::ToString;
use openssl::x509::{X509, X509VerifyResult};
use openssl::sign::Verifier;
use openssl::hash::MessageDigest;
use hex::FromHex;

#[derive(Serialize, Deserialize, Debug)]
pub struct ASReport {
    pub id : String, 
    pub timestamp : String,
    pub isvEnclaveQuoteStatus : String,
    pub platformInfoBlob : String,
    pub isvEnclaveQuoteBody : String
}
#[derive(Serialize, Deserialize, Debug)]
pub struct ASResult {
    pub ca : String, 
    pub certificate : String,
    pub report : ASReport,
    pub report_string : String,
    pub signature : String, 
    pub validate : bool
}
#[derive(Serialize, Deserialize, Debug)]
pub struct ASResponse {
    pub id : i64,
    pub jsonrpc : String, 
    pub result : ASResult
}
#[derive(Serialize, Deserialize, Debug)]
pub struct Params {
    pub quote : String,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct QuoteRequest {
    pub jsonrpc : String, 
    pub method : String, 
    pub params : Params, 
    pub id : i32, 
}

#[derive(Default)]
pub struct Quote {
    pub body: QBody,
    pub report_body: QReportBody,
}

pub struct QBody { // size: 48
    pub version: [u8; 2],
    pub signature_type: [u8; 2],
    pub gid: [u8; 4],
    pub isv_svn_qe: [u8; 2],
    pub isv_svn_pce: [u8; 2],
    pub reserved: [u8; 4],
    pub base_name: [u8; 32],
}

pub struct QReportBody { // size: 384
    pub cpu_svn: [u8; 16],
    pub misc_select: [u8; 4],
    pub reserved: [u8; 28],
    pub attributes: [u8; 16],
    pub mr_enclave: [u8; 32],
    pub reserved2: [u8; 32],
    pub mr_signer: [u8; 32],
    pub reserved3: [u8; 96],
    pub isv_prod_id: [u8; 2],
    pub isv_svn: [u8; 2],
    pub reserved4: [u8; 60],
    pub report_data: [u8; 64],
}

pub struct AttestationService {
    connection_str : String,
}
impl AttestationService{
    pub fn new(conn_str: &str)->AttestationService{
        AttestationService{
            connection_str : conn_str.to_string()
        }
    }
    pub fn get_report(&self,quote : &String)-> Result<ASResponse,Error>{
        let request : QuoteRequest = self.build_request(quote);
        let response : ASResponse =  self.send_request(request)?;
        Ok(response)
    }
    // input: encrypted enclave quote 
    // output : JSON-RPC request object
    pub fn build_request(&self, quote : &String) -> QuoteRequest{
        QuoteRequest{
            jsonrpc : "2.0".to_string(),
            method : "validate".to_string(),
            params : Params{
                quote : quote.to_string(),
            },
            id : 1
        }
    }
    // request the report object 
    pub fn send_request(&self,quote_req : QuoteRequest)-> Result<ASResponse,Error>{
        
        let client = reqwest::Client::new();
        let mut res = client.post(self.connection_str.as_str())
            .json(&quote_req)
            .send().unwrap();
        let response_str =  res.text().unwrap();
        let json_response : Value = serde_json::from_str(response_str.as_str()).unwrap();

        if res.status().is_success(){
            // parse the Json object into an ASResponse struct 
            let response : ASResponse = self.unwrap_response(&json_response);
            Ok(response)

        }else if res.status().is_server_error(){

             let mut message = String::from("[-] AttestationService: Server Error happened. Status code: ");
             message.push_str(res.status().to_string().as_str());
             Err(errors::AttestationServiceErr{
                 message : message
             }.into())

        }else{

             let mut message = String::from("[-] AttestationService: Unkown Error happened. Status code: ");
             message.push_str(res.status().to_string().as_str());
             Err(errors::AttestationServiceErr{
                 message : message
             }.into())

        }
}
    // encode to rlp the report -> registration for the enigma contract 
    pub fn rlp_encode_registration_params(&self,quote : &String)->Result<(Vec<u8>,ASResponse),Error>{
        let as_response = self.get_report(quote).unwrap();
        // certificate,signature,report_string are all need to be rlp encoded and send to register() func in enigma contract
        let certificate = as_response.result.certificate.clone();
        let signature = as_response.result.signature.clone();
        let report_string = as_response.result.report_string.clone();
        // rlp encoding 
        let clear = vec![report_string.as_str(), certificate.as_str(), signature.as_str()];
        let encoded = rlp::encode_list::<&str,&str>(&clear).to_vec();
        Ok((encoded,as_response))
    }
    // parse the response json into an ASResponse
    fn unwrap_report_obj(&self,r : &Value) -> ASReport {
        let report_str = r["result"]["report"].as_str().unwrap();
        let report_obj : ASReport = serde_json::from_str(report_str).unwrap();
        report_obj
    }

    fn unwrap_result(&self,r : & Value) -> ASResult{
        let ca = r["result"]["ca"].as_str().unwrap();
        let certificate = r["result"]["certificate"].as_str().unwrap();
        let signature = r["result"]["signature"].as_str().unwrap();
        let validate = match r["result"]["validate"].as_str() {
            Some(v)=>{
                if v == "True"{
                    true
                }else{
                    false
                }
            },
            None =>{
                false
            },
        };
        let report : ASReport =  self.unwrap_report_obj(r);
        let result_obj : ASResult  = ASResult{
            ca: ca.to_string(), 
            certificate : certificate.to_string(), 
            signature : signature.to_string(), 
            validate : validate , 
            report : report , 
            report_string :  r["result"]["report"].as_str().unwrap().to_string()
        };
        result_obj
    }

    fn unwrap_response(&self,r : & Value) -> ASResponse{
        let result : ASResult = self.unwrap_result(r);
        let id = r["id"].as_i64().unwrap();
        let jsonrpc = r["jsonrpc"].as_str().unwrap();
        let response_obj : ASResponse = ASResponse {
            id : id , 
            jsonrpc : jsonrpc.to_string(),
            result : result
        };
        response_obj
    }

}

impl ASResponse {
    pub fn get_quote(&self) -> Result<Quote, Error> {
        Quote::from_base64(&self.result.report.isvEnclaveQuoteBody)
    }
}

impl ASResult {
    pub fn verify_report(&self) -> Result<bool, Error> {
        let ca = X509::from_pem(&self.ca.as_bytes())?;
        let cert = X509::from_pem(&self.certificate.as_bytes())?;
        match ca.issued(&cert) {
            X509VerifyResult::OK => (),
            _ => return Ok(false),
        };
        let pubkey = cert.public_key()?;
        let sig = self.signature.from_hex()?;
        let mut verifier = Verifier::new(MessageDigest::sha256(), &pubkey)?;
        verifier.update(&self.report_string.as_bytes())?;
        Ok(verifier.verify(&sig)?)
    }
}

impl Quote {
    pub fn from_base64(encoded_quote: &str) -> Result<Quote, Error> {
        let quote_bytes =  base64::decode(encoded_quote)?;
        let mut result: Quote = Default::default();
//        let mut sig_len = [0u8; 4]; sig_len.copy_from_slice(&quote_bytes[432..436]);
        Ok(Quote {
            body: QBody::from_bytes_read(&mut &quote_bytes[..48])?,
            report_body: QReportBody::from_bytes_read(&mut &quote_bytes[48..432])?,
        })
    }
}


impl QBody {
    pub fn from_bytes_read<R: Read> (body: &mut R) -> Result<QBody, Error> {
        let mut result: QBody = Default::default();

        body.read_exact(&mut result.version)?;
        body.read_exact(&mut result.signature_type)?;
        body.read_exact(&mut result.gid)?;
        body.read_exact(&mut result.isv_svn_qe)?;
        body.read_exact(&mut result.isv_svn_pce)?;
        body.read_exact(&mut result.reserved)?;
        body.read_exact(&mut result.base_name)?;

        if body.read(&mut [0u8])? != 0 {
            return Err( errors::QuoteErr { message: "String passed to QBody is too big".to_string() }.into() )
        }
        Ok(result)
    }
}

impl Default for QBody {
    fn default() -> QBody {
        unsafe { mem::zeroed() }
    }
}

impl QReportBody { // Size: 384
    pub fn from_bytes_read<R: Read> (body: &mut R) -> Result<QReportBody, Error> {
        let mut result: QReportBody = Default::default();

        body.read_exact(&mut result.cpu_svn)?;
        body.read_exact(&mut result.misc_select)?;
        body.read_exact(&mut result.reserved)?;
        body.read_exact(&mut result.attributes)?;
        body.read_exact(&mut result.mr_enclave)?;
        body.read_exact(&mut result.reserved2)?;
        body.read_exact(&mut result.mr_signer)?;
        body.read_exact(&mut result.reserved3)?;
        body.read_exact(&mut result.isv_prod_id)?;
        body.read_exact(&mut result.isv_svn)?;
        body.read_exact(&mut result.reserved4)?;
        body.read_exact(&mut result.report_data)?;

        if body.read(&mut [0u8])? != 0 {
            return Err( errors::QuoteErr { message: "String passed to QReportBody is too big".to_string() }.into() )
        }
        Ok(result)
    }
}

impl Default for QReportBody {
    fn default() -> QReportBody {
        unsafe { mem::zeroed() }
    }
}


 #[cfg(test)]  
 mod test {
     use attestation_service::service::*;
     use attestation_service;
     use std::str::from_utf8;
    // this unit-test is for the attestation service
    // it uses a hardcoded quote that is validated 
    // the test requests a report from the attestation service construct an object with the response 
    // for signing the report there's additional field that can be accessed via ASResponse.result.report_string
     #[test]
     fn test_get_response_attestation_service(){ 
        // build a request 
        let service : AttestationService = AttestationService::new(attestation_service::constants::ATTESTATION_SERVICE_URL);
        let quote = String::from("AgAAANoKAAAHAAYAAAAAABYB+Vw5ueowf+qruQGtw+54eaWW7MiyrIAooQw/uU3eBAT/////AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABwAAAAAAAAAHAAAAAAAAALcVy53ugrfvYImaDi1ZW5RueQiEekyu/HmLIKYvg6OxAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACD1xnnferKFHD2uvYqTXdDA8iZ22kCD5xw7h38CMfOngAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACGcCDM4cgbYe6zQSwWQINFsDvd21kXGeteDakovCXPDwjJ31WG0K+wyDDRo8PFi293DtIr6DgNqS/guQSkglPJqAIAALbvs91Ugh9/yhBpAyGQPth+UWXRboGkTaZ3DY8U+Upkb2NWbLPkXbcMbB7c3SAfV4ip/kPswyq0OuTTiJijsUyOBOV3hVLIWM4f2wVXwxiVRXrfeFs/CGla6rGdRQpFzi4wWtrdKisVK5+Cyrt2y38Ialm0NqY9FIjxlodD9D7TC8fv0Xog29V1HROlY+PvRNa+f2qp858w8j+9TshkvOAdE1oVzu0F8KylbXfsSXhH7d+n0c8fqSBoLLEjedoDBp3KSO0bof/uzX2lGQJkZhJ/RSPPvND/1gVj9q1lTM5ccbfVfkmwdN0B5iDA5fMJaRz5o8SVILr3uWoBiwx7qsUceyGX77tCn2gZxfiOICNrpy3vv384TO2ovkwvhq1Lg071eXAlxQVtPvRYOGgBAABydn7bEWdP2htRd46nBkGIAoNAnhMvbGNbGCKtNVQAU0N9f7CROLPOTrlw9gVlKK+G5vM1X95KTdcOjs8gKtTkgEos021zBs9R+whyUcs9npo1SJ8GzowVwTwWfVz9adw2jL95zwJ/qz+y5x/IONw9iXspczf7W+bwyQpNaetO9xapF6aHg2/1w7st9yJOd0OfCZsowikJ4JRhAMcmwj4tiHovLyo2fpP3SiNGzDfzrpD+PdvBpyQgg4aPuxqGW8z+4SGn+vwadsLr+kIB4z7jcLQgkMSAplrnczr0GQZJuIPLxfk9mp8oi5dF3+jqvT1d4CWhRwocrs7Vm1tAKxiOBzkUElNaVEoFCPmUYE7uZhfMqOAUsylj3Db1zx1F1d5rPHgRhybpNpxThVWWnuT89I0XLO0WoQeuCSRT0Y9em1lsozSu2wrDKF933GL7YL0TEeKw3qFTPKsmUNlWMIow0jfWrfds/Lasz4pbGA7XXjhylwum8e/I");
        let as_response : ASResponse = service.get_report(&quote).unwrap();
        // THE report as a string ready for signing 
        //println!("report to be signed string => {}",as_response.result.report_string );
        // example on how to access some param inside ASResponse
        //println!("report isv enclave quote status  => {}",as_response.result.report.isvEnclaveQuoteStatus );
        assert_eq!(true, as_response.result.validate);
        assert_eq!("2.0",as_response.jsonrpc );
     }
     // get rlp_encoded Vec<u8> that contains the bytes array for worker registration in the enigma smart contract.
    #[test]
     fn test_get_response_attestation_service_rlp_encoded(){ 
        // build a request 
        let service : AttestationService = AttestationService::new(attestation_service::constants::ATTESTATION_SERVICE_URL);
        let quote = String::from("AgAAANoKAAAHAAYAAAAAABYB+Vw5ueowf+qruQGtw+54eaWW7MiyrIAooQw/uU3eBAT/////AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABwAAAAAAAAAHAAAAAAAAALcVy53ugrfvYImaDi1ZW5RueQiEekyu/HmLIKYvg6OxAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACD1xnnferKFHD2uvYqTXdDA8iZ22kCD5xw7h38CMfOngAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACGcCDM4cgbYe6zQSwWQINFsDvd21kXGeteDakovCXPDwjJ31WG0K+wyDDRo8PFi293DtIr6DgNqS/guQSkglPJqAIAALbvs91Ugh9/yhBpAyGQPth+UWXRboGkTaZ3DY8U+Upkb2NWbLPkXbcMbB7c3SAfV4ip/kPswyq0OuTTiJijsUyOBOV3hVLIWM4f2wVXwxiVRXrfeFs/CGla6rGdRQpFzi4wWtrdKisVK5+Cyrt2y38Ialm0NqY9FIjxlodD9D7TC8fv0Xog29V1HROlY+PvRNa+f2qp858w8j+9TshkvOAdE1oVzu0F8KylbXfsSXhH7d+n0c8fqSBoLLEjedoDBp3KSO0bof/uzX2lGQJkZhJ/RSPPvND/1gVj9q1lTM5ccbfVfkmwdN0B5iDA5fMJaRz5o8SVILr3uWoBiwx7qsUceyGX77tCn2gZxfiOICNrpy3vv384TO2ovkwvhq1Lg071eXAlxQVtPvRYOGgBAABydn7bEWdP2htRd46nBkGIAoNAnhMvbGNbGCKtNVQAU0N9f7CROLPOTrlw9gVlKK+G5vM1X95KTdcOjs8gKtTkgEos021zBs9R+whyUcs9npo1SJ8GzowVwTwWfVz9adw2jL95zwJ/qz+y5x/IONw9iXspczf7W+bwyQpNaetO9xapF6aHg2/1w7st9yJOd0OfCZsowikJ4JRhAMcmwj4tiHovLyo2fpP3SiNGzDfzrpD+PdvBpyQgg4aPuxqGW8z+4SGn+vwadsLr+kIB4z7jcLQgkMSAplrnczr0GQZJuIPLxfk9mp8oi5dF3+jqvT1d4CWhRwocrs7Vm1tAKxiOBzkUElNaVEoFCPmUYE7uZhfMqOAUsylj3Db1zx1F1d5rPHgRhybpNpxThVWWnuT89I0XLO0WoQeuCSRT0Y9em1lsozSu2wrDKF933GL7YL0TEeKw3qFTPKsmUNlWMIow0jfWrfds/Lasz4pbGA7XXjhylwum8e/I");
        let (rlp_encoded, as_response ) = service.rlp_encode_registration_params(&quote).unwrap();
        assert_eq!(true, as_response.result.validate);
        assert_eq!("2.0",as_response.jsonrpc );
     }
     #[test]
     fn test_decoding_quote() {
         let isv_enclave_quote = "AgAAANoKAAAHAAYAAAAAABYB+Vw5ueowf+qruQGtw+5gbJslhOX9eWDNazWpHhBVBAT/////AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABwAAAAAAAAAHAAAAAAAAABIhP23bLUNSZ1yvFIrZa0pu/zt6/n3X8qNjMVbWgOGDAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACD1xnnferKFHD2uvYqTXdDA8iZ22kCD5xw7h38CMfOngAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAweDRlNmRkMjg0NzdkM2NkY2QzMTA3NTA3YjYxNzM3YWFhMTU5MTYwNzAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
         let quote = Quote::from_base64(&isv_enclave_quote).unwrap();
         let data = quote.report_body.report_data;
         let data_str = from_utf8(&data).unwrap();
         assert_eq!(data_str.trim_right_matches("\x00"), "0x4e6dd28477d3cdcd3107507b61737aaa15916070");
     }

     #[test]
     fn test_verify_report() {
         let report = ASResult {
             ca: "-----BEGIN CERTIFICATE-----\nMIIFSzCCA7OgAwIBAgIJANEHdl0yo7CUMA0GCSqGSIb3DQEBCwUAMH4xCzAJBgNV\nBAYTAlVTMQswCQYDVQQIDAJDQTEUMBIGA1UEBwwLU2FudGEgQ2xhcmExGjAYBgNV\nBAoMEUludGVsIENvcnBvcmF0aW9uMTAwLgYDVQQDDCdJbnRlbCBTR1ggQXR0ZXN0\nYXRpb24gUmVwb3J0IFNpZ25pbmcgQ0EwIBcNMTYxMTE0MTUzNzMxWhgPMjA0OTEy\nMzEyMzU5NTlaMH4xCzAJBgNVBAYTAlVTMQswCQYDVQQIDAJDQTEUMBIGA1UEBwwL\nU2FudGEgQ2xhcmExGjAYBgNVBAoMEUludGVsIENvcnBvcmF0aW9uMTAwLgYDVQQD\nDCdJbnRlbCBTR1ggQXR0ZXN0YXRpb24gUmVwb3J0IFNpZ25pbmcgQ0EwggGiMA0G\nCSqGSIb3DQEBAQUAA4IBjwAwggGKAoIBgQCfPGR+tXc8u1EtJzLA10Feu1Wg+p7e\nLmSRmeaCHbkQ1TF3Nwl3RmpqXkeGzNLd69QUnWovYyVSndEMyYc3sHecGgfinEeh\nrgBJSEdsSJ9FpaFdesjsxqzGRa20PYdnnfWcCTvFoulpbFR4VBuXnnVLVzkUvlXT\nL/TAnd8nIZk0zZkFJ7P5LtePvykkar7LcSQO85wtcQe0R1Raf/sQ6wYKaKmFgCGe\nNpEJUmg4ktal4qgIAxk+QHUxQE42sxViN5mqglB0QJdUot/o9a/V/mMeH8KvOAiQ\nbyinkNndn+Bgk5sSV5DFgF0DffVqmVMblt5p3jPtImzBIH0QQrXJq39AT8cRwP5H\nafuVeLHcDsRp6hol4P+ZFIhu8mmbI1u0hH3W/0C2BuYXB5PC+5izFFh/nP0lc2Lf\n6rELO9LZdnOhpL1ExFOq9H/B8tPQ84T3Sgb4nAifDabNt/zu6MmCGo5U8lwEFtGM\nRoOaX4AS+909x00lYnmtwsDVWv9vBiJCXRsCAwEAAaOByTCBxjBgBgNVHR8EWTBX\nMFWgU6BRhk9odHRwOi8vdHJ1c3RlZHNlcnZpY2VzLmludGVsLmNvbS9jb250ZW50\nL0NSTC9TR1gvQXR0ZXN0YXRpb25SZXBvcnRTaWduaW5nQ0EuY3JsMB0GA1UdDgQW\nBBR4Q3t2pn680K9+QjfrNXw7hwFRPDAfBgNVHSMEGDAWgBR4Q3t2pn680K9+Qjfr\nNXw7hwFRPDAOBgNVHQ8BAf8EBAMCAQYwEgYDVR0TAQH/BAgwBgEB/wIBADANBgkq\nhkiG9w0BAQsFAAOCAYEAeF8tYMXICvQqeXYQITkV2oLJsp6J4JAqJabHWxYJHGir\nIEqucRiJSSx+HjIJEUVaj8E0QjEud6Y5lNmXlcjqRXaCPOqK0eGRz6hi+ripMtPZ\nsFNaBwLQVV905SDjAzDzNIDnrcnXyB4gcDFCvwDFKKgLRjOB/WAqgscDUoGq5ZVi\nzLUzTqiQPmULAQaB9c6Oti6snEFJiCQ67JLyW/E83/frzCmO5Ru6WjU4tmsmy8Ra\nUd4APK0wZTGtfPXU7w+IBdG5Ez0kE1qzxGQaL4gINJ1zMyleDnbuS8UicjJijvqA\n152Sq049ESDz+1rRGc2NVEqh1KaGXmtXvqxXcTB+Ljy5Bw2ke0v8iGngFBPqCTVB\n3op5KBG3RjbF6RRSzwzuWfL7QErNC8WEy5yDVARzTA5+xmBc388v9Dm21HGfcC8O\nDD+gT9sSpssq0ascmvH49MOgjt1yoysLtdCtJW/9FZpoOypaHx0R+mJTLwPXVMrv\nDaVzWh5aiEx+idkSGMnX\n-----END CERTIFICATE-----".to_string(),
             certificate: "-----BEGIN CERTIFICATE-----\nMIIEoTCCAwmgAwIBAgIJANEHdl0yo7CWMA0GCSqGSIb3DQEBCwUAMH4xCzAJBgNV\nBAYTAlVTMQswCQYDVQQIDAJDQTEUMBIGA1UEBwwLU2FudGEgQ2xhcmExGjAYBgNV\nBAoMEUludGVsIENvcnBvcmF0aW9uMTAwLgYDVQQDDCdJbnRlbCBTR1ggQXR0ZXN0\nYXRpb24gUmVwb3J0IFNpZ25pbmcgQ0EwHhcNMTYxMTIyMDkzNjU4WhcNMjYxMTIw\nMDkzNjU4WjB7MQswCQYDVQQGEwJVUzELMAkGA1UECAwCQ0ExFDASBgNVBAcMC1Nh\nbnRhIENsYXJhMRowGAYDVQQKDBFJbnRlbCBDb3Jwb3JhdGlvbjEtMCsGA1UEAwwk\nSW50ZWwgU0dYIEF0dGVzdGF0aW9uIFJlcG9ydCBTaWduaW5nMIIBIjANBgkqhkiG\n9w0BAQEFAAOCAQ8AMIIBCgKCAQEAqXot4OZuphR8nudFrAFiaGxxkgma/Es/BA+t\nbeCTUR106AL1ENcWA4FX3K+E9BBL0/7X5rj5nIgX/R/1ubhkKWw9gfqPG3KeAtId\ncv/uTO1yXv50vqaPvE1CRChvzdS/ZEBqQ5oVvLTPZ3VEicQjlytKgN9cLnxbwtuv\nLUK7eyRPfJW/ksddOzP8VBBniolYnRCD2jrMRZ8nBM2ZWYwnXnwYeOAHV+W9tOhA\nImwRwKF/95yAsVwd21ryHMJBcGH70qLagZ7Ttyt++qO/6+KAXJuKwZqjRlEtSEz8\ngZQeFfVYgcwSfo96oSMAzVr7V0L6HSDLRnpb6xxmbPdqNol4tQIDAQABo4GkMIGh\nMB8GA1UdIwQYMBaAFHhDe3amfrzQr35CN+s1fDuHAVE8MA4GA1UdDwEB/wQEAwIG\nwDAMBgNVHRMBAf8EAjAAMGAGA1UdHwRZMFcwVaBToFGGT2h0dHA6Ly90cnVzdGVk\nc2VydmljZXMuaW50ZWwuY29tL2NvbnRlbnQvQ1JML1NHWC9BdHRlc3RhdGlvblJl\ncG9ydFNpZ25pbmdDQS5jcmwwDQYJKoZIhvcNAQELBQADggGBAGcIthtcK9IVRz4r\nRq+ZKE+7k50/OxUsmW8aavOzKb0iCx07YQ9rzi5nU73tME2yGRLzhSViFs/LpFa9\nlpQL6JL1aQwmDR74TxYGBAIi5f4I5TJoCCEqRHz91kpG6Uvyn2tLmnIdJbPE4vYv\nWLrtXXfFBSSPD4Afn7+3/XUggAlc7oCTizOfbbtOFlYA4g5KcYgS1J2ZAeMQqbUd\nZseZCcaZZZn65tdqee8UXZlDvx0+NdO0LR+5pFy+juM0wWbu59MvzcmTXbjsi7HY\n6zd53Yq5K244fwFHRQ8eOB0IWB+4PfM7FeAApZvlfqlKOlLcZL2uyVmzRkyR5yW7\n2uo9mehX44CiPJ2fse9Y6eQtcfEhMPkmHXI01sN+KwPbpA39+xOsStjhP9N1Y1a2\ntQAVo+yVgLgV2Hws73Fc0o3wC78qPEA+v2aRs/Be3ZFDgDyghc/1fgU+7C+P6kbq\nd4poyb6IW8KCJbxfMJvkordNOgOUUxndPHEi/tb/U7uLjLOgPA==\n-----END CERTIFICATE-----".to_string(),
             report: unsafe { mem::zeroed() },
             report_string: "{\"id\":\"100342731086430570647295023189732744265\",\"timestamp\":\"2018-07-15T16:06:47.993263\",\"isvEnclaveQuoteStatus\":\"GROUP_OUT_OF_DATE\",\"platformInfoBlob\":\"1502006504000100000505020401010000000000000000000007000006000000020000000000000ADAD85ADE5C84743B9E8ABF2638808A7597A6EEBCEAA6A041429083B3CF232D6F746C7B19C832166D8ABB60F90BCE917270555115B0050F7E65B81253F794F665AA\",\"isvEnclaveQuoteBody\":\"AgAAANoKAAAHAAYAAAAAABYB+Vw5ueowf+qruQGtw+5gbJslhOX9eWDNazWpHhBVBAT/////AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABwAAAAAAAAAHAAAAAAAAABIhP23bLUNSZ1yvFIrZa0pu/zt6/n3X8qNjMVbWgOGDAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACD1xnnferKFHD2uvYqTXdDA8iZ22kCD5xw7h38CMfOngAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAweDRlNmRkMjg0NzdkM2NkY2QzMTA3NTA3YjYxNzM3YWFhMTU5MTYwNzAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\"}".to_string(),
             signature: "9e6a05bf42a627e3066b0067dc98bc22670df0061e42eed6a5af51ffa2e3b41949b6b177980b68c43855d4df71b2817b30f54bc40566225e6b721eb21fc0aba9b58e043bfaaae320e8d9613d514c0694b36b3fe41588b15480a6f7a4d025c244af531c7145d37f8b28c223bfb46c157470246e3dbd4aa15681103df2c8fd47bb59f7b827de559992fd24260e1113912bd98ba5cd769504bb5f21471ecd4f7713f600ae5169761c9047c09d186ad91f5ff89893c13be15d11bb663099192bcf2ce81f3cbbc28c9db93ce1a4df1141372d0d738fd9d0924d1e4fe58a6e2d12a5d2f723e498b783a6355ca737c4b0feeae3285340171cbe96ade8d8b926b23a8c90".to_string(),
             validate: true,
         };
         assert!(report.verify_report().unwrap());
     }

     #[test]
     fn test_attestation_service_decode_and_verify() {
         let service : AttestationService = AttestationService::new(attestation_service::constants::ATTESTATION_SERVICE_URL);
         let encrypted_quote = "AgAAANoKAAAHAAYAAAAAABYB+Vw5ueowf+qruQGtw+5gbJslhOX9eWDNazWpHhBVBAT/////AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABwAAAAAAAAAHAAAAAAAAABIhP23bLUNSZ1yvFIrZa0pu/zt6/n3X8qNjMVbWgOGDAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACD1xnnferKFHD2uvYqTXdDA8iZ22kCD5xw7h38CMfOngAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAweDRlNmRkMjg0NzdkM2NkY2QzMTA3NTA3YjYxNzM3YWFhMTU5MTYwNzAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAqAIAAFCUk3u9iZOe6m0Hoq/2vHgHO2b4vJfI7saGcAdAYUJZ4Bu4hpXDRS+CfUSRHSn26sbSVL+1gcwx+lZZ/csjLJocAqlgyN1jkaAmMlKcX+Nz4m3ecBJCNKH++72YJyxyWLsUlY1GcjEDHhZUgJzb4z2Gl5cm8X+KWSyAqi9RVq5QEsvFQZ04ONiojOKgfBY0Y2J09lU0zOz2bKQLErSiNeJgq7bt/lu+IbDELWyqxJ0IoCOdvQapVjT5i8Rw9Y2eC3pXpR8uFlyR1e8bNXDrc7PofXttd4nJkoTrQuJpaR+d5jzIa9alLUVLPIQYCdMSNWmQ+Tv6OcO7gyhy7O5AYla+9FN8EAifAFJaE89uRfeB71TX+uP0l/XkkBQkDtEWD6H7TvjTOYGC4B3aYGgBAAD3Z5Uk/cPTTc6fn9LFdG+7445aVagObJdO3BD9+YNqPyu1j7jabltSFUxrM79lV4kt3P1BJpL+OCUQs7nob9/GkhzM5FsVc02Uj+kKnHkX9/9xSzWgP6NPMDHy5qKMEgKfrznzUyffgAzv3Mcn31S1A7cHHi5kyeQGriHDBD6+zVFMI0bqNblMwLYcJtQy0bfjDQRoqOn6YB5H2tbMpZ67QYtkhs0G6MhsPWoHW8qKIem1yjbHs0UedFZEhENrcgZyk8qHNtPndnlAeQ5gMv03W2VvRNO16QhdFL8+zEOtzxSuAq+XVHgP+eJuL4Q+ikL5+BKFc2WXNgy5PWj8bvvCfF2g7UmMJQyj0IPOlaAdjyYyTGY8lGCgN4adlfTpsAciZexR37emb8awQZkawSLWewKht9TjuLHtW/WbUKpJiSv6OF/TrTfr4Jmm6LehJ9eDQFaqkS6SPjF6Byn05t6+FPLE2XXHEqQ5v5jq2CkOalP4ftNXFyr2".to_string();
         let response = service.get_report(&encrypted_quote).unwrap();
         let quote = response.get_quote().unwrap();
         let address = from_utf8(&quote.report_body.report_data).unwrap();
         assert_eq!(address.trim_right_matches("\x00"), "0x4e6dd28477d3cdcd3107507b61737aaa15916070");
         assert!(response.result.verify_report().unwrap());
     }
 }