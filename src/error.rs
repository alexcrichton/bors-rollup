use curl;
use docopt;
use serialize::json;
use std;
use std::error::FromError;

#[deriving(Show)]
pub enum Error {
    HTTPError(curl::ErrCode),
    JSONDecoderError(json::DecoderError),
    JSONParserError(json::ParserError),
    GitError(std::io::IoError),
    DocoptError(docopt::Error)
}
impl FromError<curl::ErrCode> for Error {
    fn from_error(error: curl::ErrCode) -> Error {
        Error::HTTPError(error)
    }
}
impl FromError<json::DecoderError> for Error {
    fn from_error(error: json::DecoderError) -> Error {
        Error::JSONDecoderError(error)
    }
}
impl FromError<json::ParserError> for Error {
    fn from_error(error: json::ParserError) -> Error {
        Error::JSONParserError(error)
    }
}
impl FromError<std::io::IoError> for Error {
    fn from_error(error: std::io::IoError) -> Error {
        Error::GitError(error)
    }
}
impl FromError<docopt::Error> for Error {
    fn from_error(error: docopt::Error) -> Error {
        Error::DocoptError(error)
    }
}
