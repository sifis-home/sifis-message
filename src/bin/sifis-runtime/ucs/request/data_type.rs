pub trait DataType {
    const URI: &'static str;
}

macro_rules! impl_integer_data_type {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl DataType for $ty {
                const URI: &'static str = "http://www.w3.org/2001/XMLSchema#integer";
            }
        )+
    };
}

impl_integer_data_type!(u8, u16, u32, u64, i8, i16, i32, i64, usize, isize);

macro_rules! impl_float_data_type {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl DataType for $ty {
                const URI: &'static str = "http://www.w3.org/2001/XMLSchema#float";
            }
        )+
    };
}

impl_float_data_type!(f32, f64);

macro_rules! impl_string_data_type {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl DataType for $ty {
                const URI: &'static str = "http://www.w3.org/2001/XMLSchema#string";
            }
        )+
    };
}

impl_string_data_type!(str, String, std::borrow::Cow<'_, str>);

impl DataType for bool {
    const URI: &'static str = "http://www.w3.org/2001/XMLSchema#boolean";
}

impl<T> DataType for &T
where
    T: DataType,
{
    const URI: &'static str = T::URI;
}
