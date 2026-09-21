namespace Broken {

int still_visible(int value)
{
    if (value > 0) {
        return value;
    }
    // Deliberately malformed expression: Tree-sitter should recover inside the function.
    return value + ;
}
