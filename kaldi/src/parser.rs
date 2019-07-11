use tract_core::internal::*;

use nom::IResult;
use nom::{
    bytes::complete::*, character::complete::*, combinator::*, multi::separated_list,
    number::complete::float, sequence::*,
};

use std::collections::HashMap;

use crate::model::{Component, KaldiProtoModel};

mod config_lines;
mod descriptor;

pub fn nnet3(slice: &[u8]) -> TractResult<KaldiProtoModel> {
    let (_, (config, components)) = parse_top_level(slice).map_err(|e| match e {
        nom::Err::Error(err) => format!("Parsing kaldi enveloppe at: {:?}", err),
        e => format!("{:?}", e),
    })?;
    let config_lines = config_lines::parse_config(config)?;
    Ok(KaldiProtoModel { config_lines, components })
}

fn parse_top_level(i: &[u8]) -> IResult<&[u8], (&str, HashMap<String, Component>)> {
    let (i, _) = open(i, "Nnet3")?;
    let (i, config_lines) = map_res(take_until("<NumComponents>"), std::str::from_utf8)(i)?;
    let (i, num_components) = num_components(i)?;
    let mut components = HashMap::new();
    let mut i = i;
    for _ in 0..num_components {
        let (new_i, (name, op)) = pair(component_name, component)(i)?;
        i = new_i;
        components.insert(name.to_owned(), op);
    }
    let (i, _) = close(i, "Nnet3")?;
    Ok((i, (config_lines, components)))
}

fn num_components(i: &[u8]) -> IResult<&[u8], usize> {
    let (i, _) = open(i, "NumComponents")?;
    let (i, n) = multispaced(integer)(i)?;
    Ok((i, n as usize))
}

fn component(i: &[u8]) -> IResult<&[u8], Component> {
    let (i, klass) = open_any(i)?;
    let (i, attributes) = nom::multi::many0(map(pair(open_any, tensor), |(k, v)| {
        (k.to_string(), v.into_arc_tensor())
    }))(i)?;
    let attributes = attributes.into_iter().collect();
    let (i, _) = close(i, klass)?;
    Ok((i, Component { klass: klass.to_string(), attributes }))
}

fn component_name(i: &[u8]) -> IResult<&[u8], &str> {
    multispaced(delimited(|i| open(i, "ComponentName"), name, multispace0))(i)
}

pub fn open<'a>(i: &'a [u8], t: &str) -> IResult<&'a [u8], ()> {
    map(multispaced(tuple((tag("<"), tag(t.as_bytes()), tag(">")))), |_| ())(i)
}

pub fn close<'a>(i: &'a [u8], t: &str) -> IResult<&'a [u8], ()> {
    map(multispaced(tuple((tag("</"), tag(t.as_bytes()), tag(">")))), |_| ())(i)
}

pub fn open_any(i: &[u8]) -> IResult<&[u8], &str> {
    multispaced(delimited(tag("<"), name, tag(">")))(i)
}

pub fn name(i: &[u8]) -> IResult<&[u8], &str> {
    map_res(
        recognize(pair(
            alpha1,
            nom::multi::many0(nom::branch::alt((alphanumeric1, tag("."), tag("_"), tag("-")))),
        )),
        std::str::from_utf8,
    )(i)
}

pub fn tensor(i: &[u8]) -> IResult<&[u8], Tensor> {
    nom::branch::alt((scalar, vector, matrix))(i)
}

pub fn matrix(i: &[u8]) -> IResult<&[u8], Tensor> {
    let (i, v) = delimited(
        multispaced(tag("[")),
        separated_list(spaced(tag("\n")), separated_list(space1, float)),
        multispaced(tag("]")),
    )(i)?;
    let lines = v.len();
    let data: Vec<_> = v.into_iter().flat_map(|v| v.into_iter()).collect();
    let cols = data.len() / lines;
    let t = tract_core::ndarray::Array1::from_vec(data);
    let t = t.into_shape((lines, cols)).unwrap();
    Ok((i, t.into_tensor()))
}

pub fn vector(i: &[u8]) -> IResult<&[u8], Tensor> {
    map(delimited(spaced(tag("[")), separated_list(space1, float), spaced(tag("]"))), |t| {
        tensor1(&*t)
    })(i)
}

pub fn scalar(i: &[u8]) -> IResult<&[u8], Tensor> {
    nom::branch::alt((
        map(float, Tensor::from),
        map(integer, Tensor::from),
        map(tag("F"), |_| Tensor::from(false)),
        map(tag("T"), |_| Tensor::from(true)),
    ))(i)
}

pub fn integer(i: &[u8]) -> IResult<&[u8], i32> {
    map_res(
        map_res(
            recognize(pair(opt(tag("-")), take_while(nom::character::is_digit))),
            std::str::from_utf8,
        ),
        |s| s.parse::<i32>(),
    )(i)
}

pub fn spaced<I, O, E: nom::error::ParseError<I>, F>(it: F) -> impl Fn(I) -> nom::IResult<I, O, E>
where
    I: nom::InputTakeAtPosition,
    <I as nom::InputTakeAtPosition>::Item: nom::AsChar + Clone,
    F: Fn(I) -> nom::IResult<I, O, E>,
{
    delimited(space0, it, space0)
}

pub fn multispaced<I, O, E: nom::error::ParseError<I>, F>(
    it: F,
) -> impl Fn(I) -> nom::IResult<I, O, E>
where
    I: nom::InputTakeAtPosition,
    <I as nom::InputTakeAtPosition>::Item: nom::AsChar + Clone,
    F: Fn(I) -> nom::IResult<I, O, E>,
{
    delimited(multispace0, it, multispace0)
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    #[test]
    fn test_nnet3_1() {
        let slice = r#"<Nnet3>

input-node name=input dim=3
component-node name=fixed1 input=input component=fixed1
output-node name=output input=fixed1

<NumComponents> 1
<ComponentName> foo <FixedAffineComponent> <LinearParams> [
  1.0 2.0 3.0
  4.0 5.0 6.0 ]
<BiasParams> [ 7.0 8.0 ]
</FixedAffineComponent>
</Nnet3>"#;
        nnet3(slice.as_bytes()).unwrap();
    }

    #[test]
    fn test_vector() {
        let slice = r#"[ 7.0 8.0 ]"#;
        assert_eq!(
            tensor(slice.as_bytes()).unwrap().1,
            tract_core::internal::tensor1(&[7.0f32, 8.0])
        );
    }

    #[test]
    fn test_matrix() {
        let slice = r#"[
            1.0 2.0 3.0
            4.0 5.0 6.0 ]"#;
        assert_eq!(
            tensor(slice.as_bytes()).unwrap().1,
            tract_core::internal::tensor2(&[[1.0f32, 2.0, 3.0], [4.0, 5.0, 6.0]])
        );
    }

    #[test]
    fn fixed_affine_40x10_T40_S3() {
        let slice = std::fs::read("test_cases/fixed_affine_40x10_T40_S3/model.raw.txt").unwrap();
        nnet3(&slice).unwrap();
    }
}
